use super::expansion::CheckConfigSource;
use crate::repo_inspection::RepoInspectionCache;
use minijinja::Environment;
use saphyr_parser::{Event, Parser, ScalarStyle, Span, Tag};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub(super) fn expand_foreach_yaml(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: &CheckConfigSource,
    cache: Arc<Mutex<RepoInspectionCache>>,
) -> Result<String, String> {
    let documents = parse_documents(content)?;
    let next_anchor = documents
        .iter()
        .map(max_anchor)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| "too many YAML anchors".to_string())?;
    let mut expansion = ForeachExpansion {
        root,
        config_path,
        source,
        cache,
        next_anchor,
    };
    let documents = documents
        .into_iter()
        .map(|document| expansion.expand_node(document, None))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(render_documents(&documents))
}

struct ForeachExpansion<'a> {
    root: &'a Path,
    config_path: &'a Path,
    source: &'a CheckConfigSource,
    cache: Arc<Mutex<RepoInspectionCache>>,
    next_anchor: usize,
}

impl ForeachExpansion<'_> {
    fn expand_node(
        &mut self,
        mut node: YamlNode,
        inherited_bindings: Option<&BTreeMap<String, Value>>,
    ) -> Result<YamlNode, String> {
        if node.has_local_tag("foreach") {
            return self.expand_foreach(node, inherited_bindings);
        }
        match &mut node.kind {
            YamlNodeKind::Sequence(items) => {
                let expanded = std::mem::take(items)
                    .into_iter()
                    .map(|item| self.expand_node(item, inherited_bindings))
                    .collect::<Result<Vec<_>, _>>()?;
                *items = expanded;
            }
            YamlNodeKind::Mapping(entries) => {
                let expanded = std::mem::take(entries)
                    .into_iter()
                    .map(|(key, value)| {
                        Ok((
                            self.expand_node(key, inherited_bindings)?,
                            self.expand_node(value, inherited_bindings)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                *entries = expanded;
            }
            YamlNodeKind::Scalar(YamlScalar::String(value)) => {
                if let Some(bindings) = inherited_bindings {
                    render_string_scalar(
                        value,
                        bindings,
                        self.root,
                        self.config_path,
                        self.source,
                        &self.cache,
                    )?;
                }
            }
            YamlNodeKind::Scalar(YamlScalar::Other(_)) | YamlNodeKind::Alias(_) => {}
        }
        Ok(node)
    }

    fn expand_foreach(
        &mut self,
        node: YamlNode,
        inherited_bindings: Option<&BTreeMap<String, Value>>,
    ) -> Result<YamlNode, String> {
        let YamlNode {
            anchor,
            kind: YamlNodeKind::Sequence(mut items),
            ..
        } = node
        else {
            return Err("!foreach must tag a two-item sequence".to_string());
        };
        if items.len() != 2 {
            return Err("!foreach must tag a two-item sequence".to_string());
        }
        let template = items.pop().expect("length checked");
        let binding_node = items.pop().expect("length checked");
        let bindings = self.foreach_bindings(binding_node)?;
        let combinations = foreach_combinations(bindings)?;
        let mut rendered = Vec::with_capacity(combinations.len());
        for combination in combinations {
            // xpec: s6
            // Binding choices stay literal. Inherited and local bindings join
            // only while rendering this template; local names shadow outer names.
            let mut bindings = inherited_bindings.cloned().unwrap_or_default();
            bindings.extend(combination);
            let mut copy = template.clone();
            freshen_anchors(&mut copy, &mut self.next_anchor)?;
            rendered.push(self.expand_node(copy, Some(&bindings))?);
        }
        Ok(YamlNode {
            anchor,
            tag: None,
            kind: YamlNodeKind::Sequence(rendered),
        })
    }

    fn foreach_bindings(&mut self, node: YamlNode) -> Result<Vec<ForeachBinding>, String> {
        let node = resolve_aliases(node, &mut HashMap::new())?;
        let YamlNodeKind::Mapping(entries) = node.kind else {
            return Err("the first !foreach item must be a mapping".to_string());
        };
        if entries.is_empty() {
            return Err("the first !foreach item must contain a binding".to_string());
        }
        let mut names = BTreeSet::new();
        let mut bindings = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let name = key
                .into_string_scalar()
                .ok_or_else(|| "!foreach variable names must be strings".to_string())?;
            if !names.insert(name.clone()) {
                return Err(format!("duplicate !foreach variable: {name}"));
            }
            let choices = match value.kind {
                YamlNodeKind::Sequence(items) => items,
                _ => vec![value],
            };
            let mut expanded = Vec::new();
            for choice in choices {
                if let Some(glob) = choice.string_scalar().filter(|value| is_glob(value)) {
                    let mut cache = self
                        .cache
                        .lock()
                        .map_err(|_| "!foreach cache lock is poisoned".to_string())?;
                    let paths =
                        self.source
                            .foreach_paths(&mut cache, self.root, self.config_path, glob)?;
                    expanded.extend(paths.into_iter().map(Value::String));
                } else {
                    expanded.push(yaml_literal_value(choice)?);
                }
            }
            bindings.push(ForeachBinding {
                name,
                choices: expanded,
            });
        }
        Ok(bindings)
    }
}

fn max_anchor(node: &YamlNode) -> usize {
    let child_max = match &node.kind {
        YamlNodeKind::Sequence(items) => items.iter().map(max_anchor).max().unwrap_or(0),
        YamlNodeKind::Mapping(entries) => entries
            .iter()
            .flat_map(|(key, value)| [max_anchor(key), max_anchor(value)])
            .max()
            .unwrap_or(0),
        YamlNodeKind::Alias(anchor) => *anchor,
        YamlNodeKind::Scalar(_) => 0,
    };
    node.anchor.max(child_max)
}

fn freshen_anchors(node: &mut YamlNode, next_anchor: &mut usize) -> Result<(), String> {
    let mut replacements = HashMap::new();
    collect_anchor_replacements(node, next_anchor, &mut replacements)?;
    replace_anchors(node, &replacements);
    Ok(())
}

fn collect_anchor_replacements(
    node: &YamlNode,
    next_anchor: &mut usize,
    replacements: &mut HashMap<usize, usize>,
) -> Result<(), String> {
    if node.anchor > 0 {
        let replacement = *next_anchor;
        *next_anchor = next_anchor
            .checked_add(1)
            .ok_or_else(|| "too many YAML anchors".to_string())?;
        replacements.insert(node.anchor, replacement);
    }
    match &node.kind {
        YamlNodeKind::Sequence(items) => {
            for item in items {
                collect_anchor_replacements(item, next_anchor, replacements)?;
            }
        }
        YamlNodeKind::Mapping(entries) => {
            for (key, value) in entries {
                collect_anchor_replacements(key, next_anchor, replacements)?;
                collect_anchor_replacements(value, next_anchor, replacements)?;
            }
        }
        YamlNodeKind::Scalar(_) | YamlNodeKind::Alias(_) => {}
    }
    Ok(())
}

fn replace_anchors(node: &mut YamlNode, replacements: &HashMap<usize, usize>) {
    if let Some(replacement) = replacements.get(&node.anchor) {
        node.anchor = *replacement;
    }
    match &mut node.kind {
        YamlNodeKind::Sequence(items) => {
            for item in items {
                replace_anchors(item, replacements);
            }
        }
        YamlNodeKind::Mapping(entries) => {
            for (key, value) in entries {
                replace_anchors(key, replacements);
                replace_anchors(value, replacements);
            }
        }
        YamlNodeKind::Alias(anchor) => {
            if let Some(replacement) = replacements.get(anchor) {
                *anchor = *replacement;
            }
        }
        YamlNodeKind::Scalar(_) => {}
    }
}

struct ForeachBinding {
    name: String,
    choices: Vec<Value>,
}

fn is_glob(value: &str) -> bool {
    value.contains(['*', '?'])
}

fn foreach_combinations(
    bindings: Vec<ForeachBinding>,
) -> Result<Vec<BTreeMap<String, Value>>, String> {
    let mut combinations = vec![BTreeMap::new()];
    for binding in bindings {
        let capacity = combinations
            .len()
            .checked_mul(binding.choices.len())
            .ok_or_else(|| "!foreach combination count exceeds platform limits".to_string())?;
        let mut expanded = Vec::with_capacity(capacity);
        for combination in combinations {
            for choice in &binding.choices {
                let mut copy = combination.clone();
                copy.insert(binding.name.clone(), choice.clone());
                // xpec: s6
                // Each occurrence in the selected Cartesian product is one
                // combination and contributes one copy, even when its binding
                // values equal those of another selected occurrence.
                expanded.push(copy);
            }
        }
        combinations = expanded;
    }
    Ok(combinations)
}

fn yaml_literal_value(node: YamlNode) -> Result<Value, String> {
    match node.kind {
        YamlNodeKind::Scalar(YamlScalar::String(value)) => Ok(Value::String(value)),
        YamlNodeKind::Scalar(YamlScalar::Other(value)) => Ok(value),
        YamlNodeKind::Sequence(items) => items
            .into_iter()
            .map(yaml_literal_value)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        YamlNodeKind::Mapping(entries) => {
            let mut object = serde_json::Map::new();
            for (key, value) in entries {
                let key = match key.kind {
                    YamlNodeKind::Scalar(YamlScalar::String(value)) => value,
                    YamlNodeKind::Scalar(YamlScalar::Other(value)) => value.to_string(),
                    _ => return Err("!foreach literal mapping keys must be scalars".to_string()),
                };
                object.insert(key, yaml_literal_value(value)?);
            }
            Ok(Value::Object(object))
        }
        YamlNodeKind::Alias(_) => unreachable!("!foreach binding aliases are resolved first"),
    }
}

fn add_foreach_read_function(
    environment: &mut Environment<'_>,
    root: &Path,
    config_path: &Path,
    source: &CheckConfigSource,
    read_cache: &Arc<Mutex<RepoInspectionCache>>,
) {
    let root = root.to_path_buf();
    let config_path = config_path.to_path_buf();
    let source = source.clone();
    let read_cache = Arc::clone(read_cache);
    environment.add_function("read", move |requested: String| {
        let mut cache = read_cache.lock().map_err(|_| {
            minijinja::Error::new(
                minijinja::ErrorKind::InvalidOperation,
                "!foreach read cache lock is poisoned",
            )
        })?;
        source
            .foreach_literal_file_content(&mut cache, &root, &config_path, &requested)
            .map_err(|err| {
                minijinja::Error::new(
                    minijinja::ErrorKind::InvalidOperation,
                    format!("!foreach read({requested:?}): {err}"),
                )
            })
    });
}

fn render_string_scalar(
    value: &mut String,
    combination: &BTreeMap<String, Value>,
    root: &Path,
    config_path: &Path,
    source: &CheckConfigSource,
    read_cache: &Arc<Mutex<RepoInspectionCache>>,
) -> Result<(), String> {
    let mut environment = Environment::new();
    environment.set_keep_trailing_newline(true);
    add_foreach_read_function(&mut environment, root, config_path, source, read_cache);
    let template = environment
        .template_from_str(value)
        .map_err(|err| format!("!foreach template: {err}"))?;
    *value = template
        .render(combination)
        .map_err(|err| format!("!foreach template: {err}"))?;
    Ok(())
}

fn resolve_aliases(
    mut node: YamlNode,
    anchors: &mut HashMap<usize, YamlNode>,
) -> Result<YamlNode, String> {
    if let YamlNodeKind::Alias(anchor) = &node.kind {
        return anchors
            .get(anchor)
            .cloned()
            .ok_or_else(|| format!("unknown YAML alias in !foreach bindings: {anchor}"));
    }
    match &mut node.kind {
        YamlNodeKind::Sequence(items) => {
            for item in items {
                *item = resolve_aliases(item.clone(), anchors)?;
            }
        }
        YamlNodeKind::Mapping(entries) => {
            for (key, value) in entries {
                *key = resolve_aliases(key.clone(), anchors)?;
                *value = resolve_aliases(value.clone(), anchors)?;
            }
        }
        YamlNodeKind::Scalar(_) => {}
        YamlNodeKind::Alias(_) => unreachable!("aliases return before recursive resolution"),
    }
    if node.anchor > 0 {
        let anchor = node.anchor;
        let mut anchored_value = node.clone();
        anchored_value.anchor = 0;
        anchors.insert(anchor, anchored_value);
    }
    Ok(node)
}

#[derive(Clone)]
struct YamlNode {
    anchor: usize,
    tag: Option<Tag>,
    kind: YamlNodeKind,
}

impl YamlNode {
    fn has_local_tag(&self, suffix: &str) -> bool {
        self.tag
            .as_ref()
            .is_some_and(|tag| tag.handle == "!" && tag.suffix == suffix)
    }

    fn string_scalar(&self) -> Option<&str> {
        match &self.kind {
            YamlNodeKind::Scalar(YamlScalar::String(value)) => Some(value),
            _ => None,
        }
    }

    fn into_string_scalar(self) -> Option<String> {
        match self.kind {
            YamlNodeKind::Scalar(YamlScalar::String(value)) => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone)]
enum YamlNodeKind {
    Scalar(YamlScalar),
    Sequence(Vec<YamlNode>),
    Mapping(Vec<(YamlNode, YamlNode)>),
    Alias(usize),
}

#[derive(Clone)]
enum YamlScalar {
    String(String),
    Other(Value),
}

fn parse_documents(content: &str) -> Result<Vec<YamlNode>, String> {
    let events = Parser::new_from_str(content)
        .map(|event| event.map_err(|err| err.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut index = 0;
    let mut documents = Vec::new();
    while index < events.len() {
        match &events[index].0 {
            Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart(_)
            | Event::DocumentEnd
            | Event::Nothing => index += 1,
            _ => documents.push(parse_node(content, &events, &mut index)?),
        }
    }
    Ok(documents)
}

fn parse_node(
    content: &str,
    events: &[(Event<'_>, Span)],
    index: &mut usize,
) -> Result<YamlNode, String> {
    let (event, span) = events
        .get(*index)
        .ok_or_else(|| "unexpected end of YAML document".to_string())?;
    *index += 1;
    match event {
        Event::Scalar(value, style, anchor, tag) => Ok(YamlNode {
            anchor: *anchor,
            tag: tag.as_deref().cloned(),
            kind: YamlNodeKind::Scalar(classify_scalar(
                content,
                span,
                value,
                *style,
                tag.as_deref(),
            )?),
        }),
        Event::SequenceStart(anchor, tag) => {
            let mut items = Vec::new();
            while !matches!(
                events.get(*index).map(|event| &event.0),
                Some(Event::SequenceEnd)
            ) {
                items.push(parse_node(content, events, index)?);
            }
            *index += 1;
            Ok(YamlNode {
                anchor: *anchor,
                tag: tag.as_deref().cloned(),
                kind: YamlNodeKind::Sequence(items),
            })
        }
        Event::MappingStart(anchor, tag) => {
            let mut entries = Vec::new();
            while !matches!(
                events.get(*index).map(|event| &event.0),
                Some(Event::MappingEnd)
            ) {
                let key = parse_node(content, events, index)?;
                let value = parse_node(content, events, index)?;
                entries.push((key, value));
            }
            *index += 1;
            Ok(YamlNode {
                anchor: *anchor,
                tag: tag.as_deref().cloned(),
                kind: YamlNodeKind::Mapping(entries),
            })
        }
        Event::Alias(anchor) => Ok(YamlNode {
            anchor: 0,
            tag: None,
            kind: YamlNodeKind::Alias(*anchor),
        }),
        _ => Err("unexpected YAML structure event".to_string()),
    }
}

fn classify_scalar(
    content: &str,
    span: &Span,
    value: &str,
    style: ScalarStyle,
    tag: Option<&Tag>,
) -> Result<YamlScalar, String> {
    if style != ScalarStyle::Plain || tag_is_string(tag) {
        return Ok(YamlScalar::String(value.to_string()));
    }
    let raw = span_text(content, span);
    let options = serde_saphyr::options! {
        strict_booleans: true,
    };
    match serde_saphyr::from_str_with_options::<Value>(raw, options)
        .map_err(|err| err.to_string())?
    {
        Value::String(_) => Ok(YamlScalar::String(value.to_string())),
        other => Ok(YamlScalar::Other(other)),
    }
}

fn tag_is_string(tag: Option<&Tag>) -> bool {
    match tag {
        None => false,
        Some(tag) if tag.handle == "tag:yaml.org,2002:" => tag.suffix == "str",
        Some(_) => true,
    }
}

fn span_text<'a>(content: &'a str, span: &Span) -> &'a str {
    let start = span.start.byte_offset().unwrap_or_else(|| {
        content
            .char_indices()
            .nth(span.start.index())
            .map_or(content.len(), |(index, _)| index)
    });
    let end = span.end.byte_offset().unwrap_or_else(|| {
        content
            .char_indices()
            .nth(span.end.index())
            .map_or(content.len(), |(index, _)| index)
    });
    &content[start..end]
}

fn render_documents(documents: &[YamlNode]) -> String {
    let mut output = String::new();
    for (index, document) in documents.iter().enumerate() {
        if documents.len() > 1 {
            if index > 0 {
                output.push('\n');
            }
            output.push_str("---\n");
        }
        render_node(document, &mut output);
        output.push('\n');
    }
    output
}

fn render_node(node: &YamlNode, output: &mut String) {
    render_tag(&node.tag, output);
    if node.anchor > 0 {
        output.push_str(&format!("&canon{} ", node.anchor));
    }
    match &node.kind {
        YamlNodeKind::Scalar(YamlScalar::String(value)) => {
            output.push_str(&serde_json::to_string(value).expect("strings are serializable"));
        }
        YamlNodeKind::Scalar(YamlScalar::Other(value)) => output.push_str(&value.to_string()),
        YamlNodeKind::Sequence(items) => {
            output.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                render_node(item, output);
            }
            output.push(']');
        }
        YamlNodeKind::Mapping(entries) => {
            output.push('{');
            for (index, (key, value)) in entries.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                render_node(key, output);
                output.push(':');
                render_node(value, output);
            }
            output.push('}');
        }
        YamlNodeKind::Alias(anchor) => output.push_str(&format!("*canon{}", anchor)),
    }
}

fn render_tag(tag: &Option<Tag>, output: &mut String) {
    let Some(tag) = tag else {
        return;
    };
    if tag.handle == "tag:yaml.org,2002:" {
        return;
    }
    if tag.handle == "!" {
        output.push('!');
        output.push_str(&tag.suffix);
    } else {
        output.push_str("!<");
        output.push_str(&tag.handle);
        output.push_str(&tag.suffix);
        output.push('>');
    }
    output.push(' ');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::config::load::parse_tree_check_config_content_with_root_and_default_agent_preset;
    use crate::check::config::yaml_include::parse_yaml_config_with_includes;
    use crate::git::TreeSource;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: s6
    fn foreach_renders_every_string_scalar_in_any_template_node() {
        let root = test_root("foreach-any-node");
        git(&root, &["init"]);
        fs::create_dir_all(root.join(".canon/specs")).unwrap();
        fs::write(root.join(".canon/specs/alpha.md"), "Alpha spec").unwrap();
        git(&root, &["add", ".canon/specs/alpha.md"]);

        let value = parse_yaml_config_with_includes::<Value>(
            &root,
            Path::new(".canon/check.yml"),
            r#"
values:
  - !foreach
    - path: "specs/*.md"
    - - "{{ path }}"
      - 7
      - "{{ path }}": "{{ read(path) }}"
"#,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .unwrap();

        assert_eq!(
            value,
            json!({
                "values": [[[
                        "specs/alpha.md",
                        7,
                        {"specs/alpha.md": "Alpha spec"}
                ]]]
            })
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: s6
    fn nested_foreach_renders_with_lexically_scoped_bindings() {
        let root = test_root("foreach-nested-bindings");
        git(&root, &["init"]);

        let value = parse_yaml_config_with_includes::<Value>(
            &root,
            Path::new(".canon/check.yml"),
            r#"
values: !foreach
  - outer: [a, b]
  - !foreach
    - inner: ["{{ outer }}", literal]
    - "{{ outer }}:{{ inner }}"
"#,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .unwrap();

        assert_eq!(
            value,
            json!({
                "values": [
                    ["a:{{ outer }}", "a:literal"],
                    ["b:{{ outer }}", "b:literal"]
                ]
            })
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: s6
    fn foreach_read_resolves_every_filename_from_the_document_directory() {
        let root = test_root("foreach-read-document-relative");
        git(&root, &["init"]);
        let file = root.join(".canon/includes/specs/alpha.md");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "Alpha value").unwrap();
        git(&root, &["add", ".canon/includes"]);

        let value = parse_yaml_config_with_includes::<Value>(
            &root,
            Path::new(".canon/includes/xpecs.yml"),
            r#"
value: !foreach
  - path: "specs/*.md"
  - from_binding: "{{ read(path) }}"
    from_expression: "{{ read(path ~ '') }}"
    from_literal: "{{ read('specs/alpha.md') }}"
    document_relative_name: "{{ path == 'specs/alpha.md' }}"
"#,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .unwrap();

        assert_eq!(
            value,
            json!({
                "value": [{
                    "from_binding": "Alpha value",
                    "from_expression": "Alpha value",
                    "from_literal": "Alpha value",
                    "document_relative_name": "true"
                }]
            })
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: s6
    fn foreach_gives_each_rendered_copy_distinct_anchors() {
        let root = test_root("foreach-anchors");
        git(&root, &["init"]);
        fs::create_dir_all(root.join(".canon/specs")).unwrap();
        fs::write(root.join(".canon/specs/alpha.md"), "Alpha spec").unwrap();
        fs::write(root.join(".canon/specs/beta.md"), "Beta spec").unwrap();
        git(&root, &["add", ".canon/specs"]);

        let value = parse_yaml_config_with_includes::<Value>(
            &root,
            Path::new(".canon/check.yml"),
            r#"
values:
  - !foreach
    - path: "specs/*.md"
    - value: &rendered "{{ path }}"
      alias: *rendered
"#,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .unwrap();

        assert_eq!(
            value,
            json!({
                "values": [[
                    {
                        "value": "specs/alpha.md",
                        "alias": "specs/alpha.md"
                    },
                    {
                        "value": "specs/beta.md",
                        "alias": "specs/beta.md"
                    }
                ]]
            })
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: s6,I8,3Z
    fn foreach_in_include_uses_including_document_directory_and_source() {
        let root = test_root("foreach-in-include");
        git(&root, &["init"]);
        fs::create_dir_all(root.join(".canon/includes/specs")).unwrap();
        fs::write(
            root.join(".canon/includes/xpecs.yml"),
            r#"
- !foreach
  - path: "specs/*.md"
  - q: "{{ path }}: {{ read(path) }}"
    a: "yes"
"#,
        )
        .unwrap();
        fs::write(root.join(".canon/includes/specs/alpha.md"), "Alpha spec").unwrap();
        git(
            &root,
            &[
                "add",
                ".canon/includes/xpecs.yml",
                ".canon/includes/specs/alpha.md",
            ],
        );
        let config = parse_tree_check_config_content_with_root_and_default_agent_preset(
            &root,
            Path::new(".canon/check.yml"),
            r#"
presets:
  default: {}
xpecs:
  - !include includes/xpecs.yml
"#,
            TreeSource::Staged,
            None,
            None,
        )
        .unwrap();

        assert_eq!(config.expectations.len(), 1);
        assert_eq!(config.expectations[0].q, "specs/alpha.md: Alpha spec");
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: s6
    fn foreach_preserves_each_repeated_combination_selection() {
        let root = test_root("foreach-cartesian");
        git(&root, &["init"]);
        fs::create_dir_all(root.join(".canon/specs")).unwrap();
        fs::write(root.join(".canon/specs/alpha.md"), "Alpha spec").unwrap();
        fs::write(root.join(".canon/specs/beta.md"), "Beta spec").unwrap();
        git(&root, &["add", ".canon/specs"]);

        let value = parse_yaml_config_with_includes::<Value>(
            &root,
            Path::new(".canon/check.yml"),
            r#"
values:
  - !foreach
    - path: ["specs/*.md", "specs/a*.md", "specs/alpha.md"]
      mode: [&brief "brief", *brief]
    - "{{ mode }} {{ path }}: {{ read(path) }}"
"#,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .unwrap();

        assert_eq!(
            value,
            json!({
                "values": [[
                    "brief specs/alpha.md: Alpha spec",
                    "brief specs/alpha.md: Alpha spec",
                    "brief specs/beta.md: Beta spec",
                    "brief specs/beta.md: Beta spec",
                    "brief specs/alpha.md: Alpha spec",
                    "brief specs/alpha.md: Alpha spec",
                    "brief specs/alpha.md: Alpha spec",
                    "brief specs/alpha.md: Alpha spec"
                ]]
            })
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: s6
    fn foreach_scalar_literal_is_one_choice_and_read_resolves_from_document_directory() {
        let root = test_root("foreach-literal-read");
        git(&root, &["init"]);
        fs::create_dir_all(root.join(".canon/includes")).unwrap();
        fs::write(root.join(".canon/includes/value.txt"), "Included literal").unwrap();
        git(&root, &["add", ".canon/includes/value.txt"]);

        let value = parse_yaml_config_with_includes::<Value>(
            &root,
            Path::new(".canon/includes/xpecs.yml"),
            r#"
value: !foreach
  - file: value.txt
    enabled: true
  - "{{ enabled }} {{ file }}: {{ read(file) }}"
"#,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .unwrap();

        assert_eq!(
            value,
            json!({"value": ["true value.txt: Included literal"]})
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: s6
    fn foreach_rejects_a_non_two_item_sequence() {
        let root = test_root("foreach-invalid-shape");
        let error = expand_foreach_yaml(
            &root,
            Path::new("check.yml"),
            "value: !foreach [path]\n",
            &CheckConfigSource::InPlace,
            Arc::new(Mutex::new(RepoInspectionCache::new())),
        )
        .unwrap_err();

        assert_eq!(error, "!foreach must tag a two-item sequence");
        let _ = fs::remove_dir_all(root);
    }

    fn test_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("canon-test-{}-{}-{}", name, process::id(), unique));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .unwrap();
        // xpec: s6,I8
        assert!(status.success(), "git {:?} failed", args);
    }
}
