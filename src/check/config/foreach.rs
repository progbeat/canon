use super::expansion::CheckConfigSource;
use crate::repo_inspection::RepoInspectionCache;
use minijinja::Environment;
use saphyr_parser::{Event, Parser, ScalarStyle, Span, Tag};
use serde_json::{json, Value};
use std::path::Path;

pub(super) fn expand_foreach_yaml(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: &CheckConfigSource,
    cache: &mut RepoInspectionCache,
) -> Result<String, String> {
    let documents = parse_documents(content)?;
    let mut expansion = ForeachExpansion {
        root,
        config_path,
        source,
        cache,
    };
    let documents = documents
        .into_iter()
        .map(|document| expansion.expand_node(document))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(render_documents(&documents))
}

struct ForeachExpansion<'a> {
    root: &'a Path,
    config_path: &'a Path,
    source: &'a CheckConfigSource,
    cache: &'a mut RepoInspectionCache,
}

impl ForeachExpansion<'_> {
    fn expand_node(&mut self, mut node: YamlNode) -> Result<YamlNode, String> {
        if node.has_local_tag("foreach") {
            return self.expand_foreach(node);
        }
        match &mut node.kind {
            YamlNodeKind::Sequence(items) => {
                let expanded = std::mem::take(items)
                    .into_iter()
                    .map(|item| self.expand_node(item))
                    .collect::<Result<Vec<_>, _>>()?;
                *items = expanded;
            }
            YamlNodeKind::Mapping(entries) => {
                let expanded = std::mem::take(entries)
                    .into_iter()
                    .map(|(key, value)| Ok((self.expand_node(key)?, self.expand_node(value)?)))
                    .collect::<Result<Vec<_>, String>>()?;
                *entries = expanded;
            }
            YamlNodeKind::Scalar(_) | YamlNodeKind::Alias(_) => {}
        }
        Ok(node)
    }

    fn expand_foreach(&mut self, node: YamlNode) -> Result<YamlNode, String> {
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
        let binding = items.pop().expect("length checked");
        let glob = foreach_glob(binding)?;
        let paths = self
            .cache
            .foreach_paths(self.root, self.config_path, &glob, self.source)?;
        let mut rendered = Vec::with_capacity(paths.len());
        for path in paths {
            let content =
                self.cache
                    .config_source_file_content(self.root, self.source, Path::new(&path))?;
            let mut copy = template.clone();
            render_string_scalars(&mut copy, &path, &content)?;
            rendered.push(self.expand_node(copy)?);
        }
        Ok(YamlNode {
            anchor,
            tag: None,
            kind: YamlNodeKind::Sequence(rendered),
        })
    }
}

fn foreach_glob(node: YamlNode) -> Result<String, String> {
    let YamlNodeKind::Mapping(mut entries) = node.kind else {
        return Err("the first !foreach item must map path to a glob".to_string());
    };
    if entries.len() != 1 {
        return Err("the first !foreach item must map path to a glob".to_string());
    }
    let (key, value) = entries.pop().expect("length checked");
    if key.string_scalar() != Some("path") {
        return Err("the first !foreach item must map path to a glob".to_string());
    }
    value
        .into_string_scalar()
        .ok_or_else(|| "the !foreach path glob must be a string".to_string())
}

fn render_string_scalars(node: &mut YamlNode, path: &str, content: &str) -> Result<(), String> {
    match &mut node.kind {
        YamlNodeKind::Scalar(YamlScalar::String(value)) => {
            let mut environment = Environment::new();
            environment.set_keep_trailing_newline(true);
            let readable_path = path.to_string();
            let readable_content = content.to_string();
            environment.add_function("read", move |requested: String| {
                if requested == readable_path {
                    Ok(readable_content.clone())
                } else {
                    Err(minijinja::Error::new(
                        minijinja::ErrorKind::InvalidOperation,
                        format!("!foreach may read only its bound path: {requested}"),
                    ))
                }
            });
            let template = environment
                .template_from_str(value)
                .map_err(|err| format!("!foreach template: {err}"))?;
            *value = template
                .render(json!({ "path": path }))
                .map_err(|err| format!("!foreach template: {err}"))?;
        }
        YamlNodeKind::Sequence(items) => {
            for item in items {
                render_string_scalars(item, path, content)?;
            }
        }
        YamlNodeKind::Mapping(entries) => {
            for (key, value) in entries {
                render_string_scalars(key, path, content)?;
                render_string_scalars(value, path, content)?;
            }
        }
        YamlNodeKind::Scalar(YamlScalar::Other(_)) | YamlNodeKind::Alias(_) => {}
    }
    Ok(())
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
    Other(String),
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
        other => serde_json::to_string(&other)
            .map(YamlScalar::Other)
            .map_err(|err| err.to_string()),
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
        YamlNodeKind::Scalar(YamlScalar::Other(value)) => output.push_str(value),
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

    #[test] // xpec: Mm
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
                        ".canon/specs/alpha.md",
                        7,
                        {".canon/specs/alpha.md": "Alpha spec"}
                ]]]
            })
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: Mm,I8,v7
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
        let mut cache = RepoInspectionCache::new();

        let config = parse_tree_check_config_content_with_root_and_default_agent_preset(
            &root,
            Path::new(".canon/check.yml"),
            r#"
presets:
  default: {}
xpecs:
  - !include includes/xpecs.yml
"#,
            &mut cache,
            TreeSource::Staged,
            None,
            None,
        )
        .unwrap();

        assert_eq!(config.expectations.len(), 1);
        assert_eq!(
            config.expectations[0].q,
            ".canon/includes/specs/alpha.md: Alpha spec"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: Mm
    fn foreach_rejects_a_non_two_item_sequence() {
        let root = test_root("foreach-invalid-shape");
        let mut cache = RepoInspectionCache::new();

        let error = expand_foreach_yaml(
            &root,
            Path::new("check.yml"),
            "value: !foreach [path]\n",
            &CheckConfigSource::InPlace,
            &mut cache,
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
        // xpec: Mm,I8
        assert!(status.success(), "git {:?} failed", args);
    }
}
