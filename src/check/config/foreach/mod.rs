use super::expansion::CheckConfigSource;
use super::yaml_include::resolve_include_path;
use crate::config_types::RawExpectationItem;
use crate::repo_inspection::RepoInspectionCache;
use minijinja::value::Value;
use serde::de::DeserializeOwned;
use serde::de::IgnoredAny;
use serde::Deserialize;
use serde_saphyr::{IncludeRequest, InputSource, ResolvedInclude};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

mod binding;
mod combinations;
mod template;

use binding::{resolve_foreach_bindings, ForeachBindings};
use combinations::foreach_combinations;
use template::{render_string, render_value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ForeachCheckConfig {
    #[serde(default, rename = "version")]
    _version: Option<IgnoredAny>,
    #[serde(default, rename = "presets")]
    _presets: Option<IgnoredAny>,
    #[serde(default, rename = "agent")]
    _agent: Option<IgnoredAny>,
    #[serde(alias = "xpecs")]
    expectations: RawExpectationEntry,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum RawExpectationEntry {
    Tagged(TaggedExpectationEntry),
    Items(Vec<RawExpectationEntry>),
    Item(Value),
}

#[derive(Clone)]
enum ExpandedYamlNode {
    Sequence(Vec<ExpandedYamlNode>),
    Value(Value),
}

#[derive(Clone, Deserialize)]
enum TaggedExpectationEntry {
    #[serde(rename = "foreach")]
    Foreach(ForeachParameters),
    #[serde(rename = "canon_include")]
    Include(IncludeParameters),
}

#[derive(Clone, Deserialize)]
struct ForeachParameters(ForeachBindings, Box<RawExpectationEntry>);

#[derive(Clone, Deserialize)]
struct IncludeParameters(String, String);

pub(super) fn expand_current_expectation_items(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: &CheckConfigSource,
    inspection_cache: Arc<Mutex<RepoInspectionCache>>,
) -> Result<Vec<RawExpectationItem>, String> {
    let node =
        expand_current_expectation_node(root, config_path, content, source, inspection_cache)?;
    let ExpandedYamlNode::Sequence(nodes) = node else {
        return Err("xpecs must be a sequence".to_string());
    };
    // [jM,MH] `!foreach` expansion above accepts and renders arbitrary YAML
    // nodes. This separate `xpecs` consumer then validates each resulting
    // sequence member as an expectation item, exactly as it does for a node
    // written directly in the `xpecs` sequence.
    let mut items = Vec::new();
    for node in nodes {
        collect_expectation_items(node, &mut items)?;
    }
    Ok(items)
}

fn collect_expectation_items(
    node: ExpandedYamlNode,
    output: &mut Vec<RawExpectationItem>,
) -> Result<(), String> {
    match node {
        ExpandedYamlNode::Sequence(nodes) => {
            for node in nodes {
                collect_expectation_items(node, output)?;
            }
            Ok(())
        }
        ExpandedYamlNode::Value(value) => {
            output.push(
                RawExpectationItem::deserialize(value)
                    .map_err(|err| format!("expectation item: {err}"))?,
            );
            Ok(())
        }
    }
}

fn expand_current_expectation_node(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: &CheckConfigSource,
    inspection_cache: Arc<Mutex<RepoInspectionCache>>,
) -> Result<ExpandedYamlNode, String> {
    let root_config_path = config_path
        .to_str()
        .ok_or_else(|| "config path must be valid UTF-8".to_string())?
        .to_string();
    let config: ForeachCheckConfig =
        parse_document(content, &root_config_path).map_err(|err| err.to_string())?;
    // Expansion is local to this config load and produces YAML nodes only.
    // Identity creation and persistent-state retention are later check-run stages.
    let mut expansion = CurrentConfigExpansion {
        root,
        root_config_path: root_config_path.clone(),
        source,
        inspection_cache,
        include_stack: vec![root_config_path],
        included_entries_by_path: BTreeMap::new(),
        included_nodes_by_path: BTreeMap::new(),
    };
    expansion.expand_entry(config.expectations, config_path, None)
}

struct CurrentConfigExpansion<'a> {
    root: &'a Path,
    root_config_path: String,
    source: &'a CheckConfigSource,
    inspection_cache: Arc<Mutex<RepoInspectionCache>>,
    include_stack: Vec<String>,
    included_entries_by_path: BTreeMap<String, RawExpectationEntry>,
    included_nodes_by_path: BTreeMap<String, ExpandedYamlNode>,
}

impl CurrentConfigExpansion<'_> {
    fn expand_entry(
        &mut self,
        entry: RawExpectationEntry,
        config_path: &Path,
        inherited: Option<&BTreeMap<String, Value>>,
    ) -> Result<ExpandedYamlNode, String> {
        match entry {
            RawExpectationEntry::Tagged(TaggedExpectationEntry::Foreach(parameters)) => {
                self.expand_foreach(parameters, config_path, inherited)
            }
            RawExpectationEntry::Tagged(TaggedExpectationEntry::Include(parameters)) => {
                self.expand_include(parameters, inherited)
            }
            RawExpectationEntry::Items(entries) => {
                let mut nodes = Vec::with_capacity(entries.len());
                for entry in entries {
                    nodes.push(self.expand_entry(entry, config_path, inherited)?);
                }
                Ok(ExpandedYamlNode::Sequence(nodes))
            }
            RawExpectationEntry::Item(mut value) => {
                if let Some(bindings) = inherited {
                    value = render_value(
                        value,
                        bindings,
                        self.root,
                        config_path,
                        self.source,
                        &self.inspection_cache,
                    )?;
                }
                Ok(ExpandedYamlNode::Value(value))
            }
        }
    }

    fn expand_foreach(
        &mut self,
        parameters: ForeachParameters,
        config_path: &Path,
        inherited: Option<&BTreeMap<String, Value>>,
    ) -> Result<ExpandedYamlNode, String> {
        let ForeachParameters(bindings, template) = parameters;
        let bindings = resolve_foreach_bindings(
            bindings,
            inherited,
            self.root,
            config_path,
            self.source,
            &self.inspection_cache,
        )?;
        let combinations = foreach_combinations(bindings)?;
        let mut nodes = Vec::with_capacity(combinations.len());
        for combination in combinations {
            let mut bindings = inherited.cloned().unwrap_or_default();
            bindings.extend(combination);
            nodes.push(self.expand_entry((*template).clone(), config_path, Some(&bindings))?);
        }
        Ok(ExpandedYamlNode::Sequence(nodes))
    }

    fn expand_include(
        &mut self,
        parameters: IncludeParameters,
        inherited: Option<&BTreeMap<String, Value>>,
    ) -> Result<ExpandedYamlNode, String> {
        let IncludeParameters(config_path, mut spec) = parameters;
        let config_path = Path::new(&config_path);
        if let Some(bindings) = inherited {
            spec = render_string(
                &spec,
                bindings,
                self.root,
                config_path,
                self.source,
                &self.inspection_cache,
            )?;
        }
        let current_path = config_path
            .to_str()
            .ok_or_else(|| "config path must be valid UTF-8".to_string())?;
        let path =
            resolve_include_path(Path::new(&self.root_config_path), &spec, Some(current_path))?;
        if self.include_stack.contains(&path) {
            return Err(format!("cyclic include detected: {path}"));
        }
        if inherited.is_none() {
            if let Some(node) = self.included_nodes_by_path.get(&path) {
                return Ok(node.clone());
            }
        }

        let entry = if let Some(entry) = self.included_entries_by_path.get(&path) {
            entry.clone()
        } else {
            let content = {
                let mut inspection_cache = self
                    .inspection_cache
                    .lock()
                    .map_err(|_| "config inspection cache lock is poisoned".to_string())?;
                self.source
                    .file_content(&mut inspection_cache, self.root, Path::new(&path))?
            };
            let entry: RawExpectationEntry = parse_document(&content, &path)
                .map_err(|err| format!("failed to parse included expectations {path}: {err}"))?;
            self.included_entries_by_path
                .insert(path.clone(), entry.clone());
            entry
        };
        self.include_stack.push(path.clone());
        let result = self.expand_entry(entry, Path::new(&path), inherited);
        self.include_stack.pop();
        let node = result?;
        if inherited.is_none() {
            self.included_nodes_by_path.insert(path, node.clone());
        }
        Ok(node)
    }
}

fn parse_document<T>(content: &str, config_path: &str) -> Result<T, serde_saphyr::Error>
where
    T: DeserializeOwned,
{
    let config_path = config_path.to_string();
    let options = serde_saphyr::options! {
        strict_booleans: true,
    }
    .with_include_resolver(move |request: IncludeRequest<'_>| {
        let spec = request.spec.to_string();
        let marker = format!(
            "!canon_include [{}, {}]",
            serde_json::to_string(&config_path).expect("string serialization cannot fail"),
            serde_json::to_string(&spec).expect("string serialization cannot fail"),
        );
        Ok(ResolvedInclude {
            id: format!("canon-include-marker:{config_path}:{spec}"),
            name: spec,
            source: InputSource::Text(marker),
        })
    });
    serde_saphyr::from_str_with_options(content, options)
}

#[cfg(test)]
mod tests {
    use super::super::load::parse_tree_check_config_content_with_root_and_default_agent_preset;
    use super::*;
    use crate::check::config::yaml_include::parse_raw_check_config_with_includes_and_foreach;
    use crate::git::TreeSource;
    use crate::repo_inspection::RepoInspectionCache;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: jM
    fn foreach_renders_each_combination_and_preserves_repeated_selections() {
        let root = staged_test_root("foreach-combinations");
        fs::create_dir_all(root.join(".canon/specs")).unwrap();
        fs::write(root.join(".canon/specs/alpha.md"), "Alpha spec").unwrap();
        fs::write(root.join(".canon/specs/beta.md"), "Beta spec").unwrap();
        git(&root, &["add", ".canon/specs"]);

        let config = parse_raw(
            &root,
            r#"
presets:
  default: {}
xpecs:
  - !foreach
    - path: ["specs/*.md", "specs/a*.md", "specs/alpha.md"]
      mode: [&brief "brief", *brief]
    - q: "{{ mode }} {{ path }}: {{ read(path) }}"
      a: yes
"#,
        )
        .unwrap();

        assert_eq!(config.expectations.len(), 8);
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: jM
    fn foreach_renders_literal_values_and_nested_sequences() {
        let root = staged_test_root("foreach-literals");
        let config = parse(
            &root,
            r#"
presets:
  default: {}
xpecs:
  - !foreach
    - choice:
        false: disabled
        2: second
      enabled: true
    - - q: "{{ choice[false] }} {{ choice[2] }}"
        a: yes
      - q: "{{ enabled }}"
        a: yes
"#,
        )
        .unwrap();

        assert_eq!(config.expectations[0].q, "disabled second");
        assert_eq!(config.expectations[1].q, "true");
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: jM
    fn nested_foreach_uses_lexically_scoped_bindings() {
        let root = staged_test_root("foreach-nested");
        let config = parse(
            &root,
            r#"
presets:
  default: {}
xpecs: !foreach
  - outer: [a, b]
  - !foreach
    - inner: ["{{ outer }}", literal]
    - q: "{{ outer }}:{{ inner }}"
      a: yes
"#,
        )
        .unwrap();

        assert_eq!(
            config
                .expectations
                .iter()
                .map(|expectation| expectation.q.as_str())
                .collect::<Vec<_>>(),
            ["a:a", "a:literal", "b:b", "b:literal"]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: jM,I8,MH
    fn foreach_in_include_uses_included_document_directory_and_source() {
        let root = staged_test_root("foreach-include");
        fs::create_dir_all(root.join(".canon/includes/specs")).unwrap();
        fs::write(
            root.join(".canon/includes/xpecs.yml"),
            r#"
- !foreach
  - path: "specs/*.md"
  - q: "{{ path }}: {{ read(path) }}"
    a: yes
"#,
        )
        .unwrap();
        fs::write(root.join(".canon/includes/specs/alpha.md"), "Alpha spec").unwrap();
        git(&root, &["add", ".canon/includes"]);

        let config = parse(
            &root,
            r#"
presets:
  default: {}
xpecs:
  - !include includes/xpecs.yml
"#,
        )
        .unwrap();

        assert_eq!(config.expectations.len(), 1);
        assert_eq!(config.expectations[0].q, "specs/alpha.md: Alpha spec");
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: jM,I8,d
    fn include_inside_foreach_uses_each_outer_binding() {
        let root = staged_test_root("include-foreach-bindings");
        fs::create_dir_all(root.join(".canon/includes")).unwrap();
        fs::write(
            root.join(".canon/includes/item.yml"),
            "q: '{{ name }}'\na: yes\n",
        )
        .unwrap();
        git(&root, &["add", ".canon/includes/item.yml"]);

        let config = parse(
            &root,
            r#"
presets:
  default: {}
xpecs: !foreach
- name: [alpha, beta]
- !include includes/item.yml
"#,
        )
        .unwrap();

        assert_eq!(
            config
                .expectations
                .iter()
                .map(|expectation| expectation.q.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "beta"]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: jM
    fn foreach_rejects_a_non_two_item_sequence() {
        let root = staged_test_root("foreach-invalid-shape");
        let result = parse(
            &root,
            r#"
presets:
  default: {}
xpecs:
  - !foreach [path]
"#,
        );

        assert!(result.is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: MH
    fn xpecs_value_must_expand_to_a_sequence() {
        let root = staged_test_root("xpecs-mapping");
        let result = parse_raw(
            &root,
            r#"
presets:
  default: {}
xpecs:
  q: Not a sequence
  a: yes
"#,
        );

        let err = match result {
            Ok(_) => panic!("mapping xpecs value was accepted"),
            Err(err) => err,
        };
        assert!(err.contains("xpecs must be a sequence"), "{err}");
        let _ = fs::remove_dir_all(root);
    }

    fn parse(root: &Path, content: &str) -> Result<crate::config_types::CheckConfig, String> {
        parse_tree_check_config_content_with_root_and_default_agent_preset(
            root,
            Path::new(".canon/check.yml"),
            content,
            TreeSource::Staged,
            None,
            None,
        )
    }

    fn parse_raw(
        root: &Path,
        content: &str,
    ) -> Result<crate::config_types::RawCheckConfig, String> {
        parse_raw_check_config_with_includes_and_foreach(
            root,
            Path::new(".canon/check.yml"),
            content,
            CheckConfigSource::Tree(TreeSource::Staged),
            RepoInspectionCache::new(),
        )
    }

    fn staged_test_root(name: &str) -> PathBuf {
        let root = test_root(name);
        git(&root, &["init"]);
        root
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
        // xpec: jM,I8
        assert!(status.success(), "git {:?} failed", args);
    }
}
