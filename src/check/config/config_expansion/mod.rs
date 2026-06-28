mod expansion;
mod generator;
mod include;
mod presets;
mod source;

pub(crate) use expansion::{expand_raw_check_config_with_options, CheckConfigExpansionOptions};
pub(crate) use source::CheckConfigSource;

#[cfg(test)]
mod tests {
    use super::{
        expand_raw_check_config_with_options, expansion::expand_raw_check_config,
        CheckConfigExpansionOptions, CheckConfigSource,
    };
    use crate::config_types::{CooldownConfig, ExpectationTarget, RawCheckConfig};
    use crate::git::TreeSource;
    use crate::repo_inspection::RepoInspectionCache;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn path_generator_expands_q_template_for_each_matching_file() {
        let root = test_root("path-generator-q-template");
        git(&root, &["init"]);
        fs::create_dir_all(root.join("specs/nested")).unwrap();
        fs::write(root.join("specs/root.md"), "Root spec").unwrap();
        fs::write(root.join("specs/nested/child.md"), "Nested spec").unwrap();
        fs::write(root.join("specs/nested/child.txt"), "Ignored spec").unwrap();
        git(
            &root,
            &[
                "add",
                "specs/root.md",
                "specs/nested/child.md",
                "specs/nested/child.txt",
            ],
        );
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default: {}
expectations:
  - path: "specs/**.md"
    q_template: |
      {{content}}
      ---
      Is this generated spec implemented?
    a: "yes"
"#,
        )
        .expect("parse raw check config");
        let mut cache = RepoInspectionCache::new();

        let config = expand_raw_check_config(
            Some(&root),
            Path::new("check.yml"),
            raw,
            Some(&mut cache),
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand config");

        let questions = config
            .expectations
            .iter()
            .map(|expectation| expectation.q.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            questions,
            vec![
                "Nested spec\n---\nIs this generated spec implemented?\n",
                "Root spec\n---\nIs this generated spec implemented?\n",
            ]
        );
        assert!(config
            .expectations
            .iter()
            .all(|expectation| expectation.a == "yes"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn include_cooldown_is_inherited_without_overriding_child_cooldown() {
        let root = test_root("include-cooldown-inheritance");
        git(&root, &["init"]);
        fs::create_dir_all(root.join("expects")).unwrap();
        fs::write(
            root.join("expects/included.yml"),
            r#"
- q: "Does the include cooldown apply?"
  a: "yes"
- q: "Does the child cooldown win?"
  a: "yes"
  cooldown: 1d
"#,
        )
        .unwrap();
        git(&root, &["add", "expects/included.yml"]);
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default: {}
expectations:
  - include: "expects/*.yml"
    cooldown: 7d
"#,
        )
        .expect("parse raw check config");
        let mut cache = RepoInspectionCache::new();

        let config = expand_raw_check_config(
            Some(&root),
            Path::new("check.yml"),
            raw,
            Some(&mut cache),
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand config");

        assert_eq!(config.expectations.len(), 2);
        assert_eq!(
            config.expectations[0].cooldown,
            Some(CooldownConfig::Compact("7d".to_string()))
        );
        assert_eq!(
            config.expectations[1].cooldown,
            Some(CooldownConfig::Compact("1d".to_string()))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn include_generator_fields_are_item_defaults() {
        let root = test_root("include-generator-field-defaults");
        git(&root, &["init"]);
        fs::create_dir_all(root.join("expects")).unwrap();
        fs::create_dir_all(root.join("expects/specs")).unwrap();
        fs::write(root.join("expects/specs/alpha.md"), "Alpha spec").unwrap();
        fs::write(
            root.join("expects/included.yml"),
            r#"
- q: "Does the include answer apply?"
- path: "specs/*.md"
- q_template: "Child generated: {{content}}"
"#,
        )
        .unwrap();
        git(
            &root,
            &["add", "expects/included.yml", "expects/specs/alpha.md"],
        );
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default: {}
expectations:
  - include: "expects/*.yml"
    path: "specs/*.md"
    q_template: "Inherited generated: {{content}}"
    a: "yes"
"#,
        )
        .expect("parse raw check config");
        let mut cache = RepoInspectionCache::new();

        let config = expand_raw_check_config(
            Some(&root),
            Path::new("check.yml"),
            raw,
            Some(&mut cache),
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand config");

        let questions = config
            .expectations
            .iter()
            .map(|expectation| expectation.q.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            questions,
            vec![
                "Does the include answer apply?",
                "Inherited generated: Alpha spec",
                "Child generated: Alpha spec",
            ]
        );
        assert!(config
            .expectations
            .iter()
            .all(|expectation| expectation.a == "yes"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unsupported_expectation_target_is_rejected_during_expansion() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default: {}
expectations:
  - q: "Does it pass?"
    a: "yes"
    target: whole-project
"#,
        )
        .expect("parse raw check config");

        let error = expand_raw_check_config(
            None,
            Path::new("check.yml"),
            raw,
            None,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .unwrap_err();

        assert_eq!(
            error,
            "expectation 1 target: unsupported target: whole-project"
        );
    }

    #[test]
    fn explicit_project_target_is_supported() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default: {}
expectations:
  - q: "Does it pass?"
    a: "yes"
    target: project
"#,
        )
        .expect("parse raw check config");

        let config = expand_raw_check_config(
            None,
            Path::new("check.yml"),
            raw,
            None,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand config");

        assert_eq!(
            config.expectations[0].target,
            Some(ExpectationTarget::Project)
        );
    }

    #[test]
    fn expectation_diff_from_expands_and_inherits() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default: {}
expectations:
  - include: "expects.yml"
    diff-from: ":against-tree"
"#,
        )
        .expect("parse raw check config");
        let root = test_root("diff-from-inheritance");
        git(&root, &["init"]);
        fs::write(
            root.join("expects.yml"),
            r#"
- q: "Does inherited diff-from apply?"
  a: "yes"
- q: "Does child diff-from win?"
  a: "yes"
  diff-from: "HEAD~1"
"#,
        )
        .unwrap();
        git(&root, &["add", "expects.yml"]);
        let mut cache = RepoInspectionCache::new();

        let config = expand_raw_check_config(
            Some(&root),
            Path::new("check.yml"),
            raw,
            Some(&mut cache),
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand config");

        assert_eq!(config.expectations[0].diff_from, ":against-tree");
        assert_eq!(config.expectations[1].diff_from, "HEAD~1");
        assert!(config.expectations[0].diff_from_configured);
        assert!(config.expectations[1].diff_from_configured);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_agent_config_still_expands_to_default_preset() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
agent:
  model:
    primary: "legacy-primary"
    fallbacks: ["legacy-fallback"]
  thinking: high
  ignore: ["tmp/**"]
expectations:
  - q: "Does the legacy agent expand?"
    a: "yes"
"#,
        )
        .expect("parse legacy raw check config");

        let config = expand_raw_check_config(
            None,
            Path::new("check.yml"),
            raw,
            None,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand legacy config");

        assert_eq!(
            config.agent.models,
            vec!["legacy-primary".to_string(), "legacy-fallback".to_string()]
        );
        assert_eq!(config.agent.thinking, "high");
        assert_eq!(config.agent.ignore, vec!["tmp/**".to_string()]);
    }

    #[test]
    fn preset_inherits_from_named_preset_with_preset_key() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default:
    models: ["default-model"]
    thinking: medium
    ignore: ["tmp/**"]
  smart:
    preset: default
    thinking: high
expectations:
  - q: "Does the smart preset inherit?"
    a: "yes"
    preset: smart
"#,
        )
        .expect("parse raw check config");

        let config = expand_raw_check_config(
            None,
            Path::new("check.yml"),
            raw,
            None,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand config");

        let expectation = &config.expectations[0];
        assert_eq!(expectation.agent.models, vec!["default-model".to_string()]);
        assert_eq!(expectation.agent.thinking, "high");
        assert_eq!(expectation.agent.ignore, vec!["tmp/**".to_string()]);
    }

    #[test]
    fn default_agent_preset_option_only_changes_config_agent() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default:
    models: ["default-model"]
  smart:
    models: ["smart-model"]
expectations:
  - q: "Does the default expectation preset stay default?"
    a: "yes"
"#,
        )
        .expect("parse raw check config");

        let config = expand_raw_check_config_with_options(
            None,
            Path::new("check.yml"),
            raw,
            None,
            CheckConfigSource::Tree(TreeSource::Staged),
            CheckConfigExpansionOptions {
                default_agent_preset: Some("smart"),
            },
        )
        .expect("expand config");

        assert_eq!(config.agent.models, vec!["smart-model".to_string()]);
        assert_eq!(
            config.expectations[0].agent.models,
            vec!["default-model".to_string()]
        );
    }

    #[test]
    fn preset_supplies_expectation_field_defaults() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default:
    q: "Does the preset supply defaults?"
    a: "yes"
    instructions: "Use the preset instructions."
    diff-from: master
    target: diff
    cooldown: 7d
    models: ["preset-model"]
    thinking: high
expectations:
  - {}
"#,
        )
        .expect("parse raw check config");

        let config = expand_raw_check_config(
            None,
            Path::new("check.yml"),
            raw,
            None,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand config");

        let expectation = &config.expectations[0];
        assert_eq!(expectation.q, "Does the preset supply defaults?");
        assert_eq!(expectation.a, "yes");
        assert_eq!(expectation.question_context, "Use the preset instructions.");
        assert_eq!(expectation.diff_from, "master");
        assert_eq!(expectation.target, Some(ExpectationTarget::Diff));
        assert_eq!(
            expectation.cooldown,
            Some(CooldownConfig::Compact("7d".to_string()))
        );
        assert_eq!(expectation.agent.models, vec!["preset-model".to_string()]);
        assert_eq!(expectation.agent.thinking, "high");
    }

    #[test]
    fn preset_supplies_generator_field_defaults() {
        let root = test_root("preset-generator-field-defaults");
        git(&root, &["init"]);
        fs::create_dir_all(root.join("specs")).unwrap();
        fs::write(root.join("specs/alpha.md"), "Alpha spec").unwrap();
        git(&root, &["add", "specs/alpha.md"]);
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default:
    path: "specs/*.md"
    q_template: |
      {{content}}
      ---
      Is this preset-generated spec implemented?
    a: "yes"
expectations:
  - {}
"#,
        )
        .expect("parse raw check config");
        let mut cache = RepoInspectionCache::new();

        let config = expand_raw_check_config(
            Some(&root),
            Path::new("check.yml"),
            raw,
            Some(&mut cache),
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand config");

        assert_eq!(config.expectations.len(), 1);
        assert_eq!(
            config.expectations[0].q,
            "Alpha spec\n---\nIs this preset-generated spec implemented?\n"
        );
        assert_eq!(config.expectations[0].a, "yes");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn item_shape_fields_prevent_preset_shape_defaults_from_overriding_form() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default:
    include: "expects/*.yml"
    path: "specs/*.md"
    q_template: "Generated: {{content}}"
    q: "Does the preset question lose?"
    a: "no"
expectations:
  - q: "Does the explicit item stay explicit?"
    a: "yes"
"#,
        )
        .expect("parse raw check config");

        let config = expand_raw_check_config(
            None,
            Path::new("check.yml"),
            raw,
            None,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand config");

        assert_eq!(config.expectations.len(), 1);
        assert_eq!(
            config.expectations[0].q,
            "Does the explicit item stay explicit?"
        );
        assert_eq!(config.expectations[0].a, "yes");
    }

    #[test]
    fn preset_supplies_missing_fields_for_declared_explicit_items() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default:
    a: "yes"
expectations:
  - q: "Does the item question use the preset answer?"
"#,
        )
        .expect("parse raw check config");

        let config = expand_raw_check_config(
            None,
            Path::new("check.yml"),
            raw,
            None,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand config");

        assert_eq!(config.expectations.len(), 1);
        assert_eq!(
            config.expectations[0].q,
            "Does the item question use the preset answer?"
        );
        assert_eq!(config.expectations[0].a, "yes");
    }

    #[test]
    fn preset_supplies_missing_fields_for_declared_generator_items() {
        let root = test_root("preset-declared-generator-fields");
        git(&root, &["init"]);
        fs::create_dir_all(root.join("specs")).unwrap();
        fs::write(root.join("specs/alpha.md"), "Alpha spec").unwrap();
        git(&root, &["add", "specs/alpha.md"]);
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default:
    path: "specs/*.md"
    a: "yes"
expectations:
  - q_template: "Generated: {{content}}"
"#,
        )
        .expect("parse raw check config");
        let mut cache = RepoInspectionCache::new();

        let config = expand_raw_check_config(
            Some(&root),
            Path::new("check.yml"),
            raw,
            Some(&mut cache),
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand config");

        assert_eq!(config.expectations.len(), 1);
        assert_eq!(config.expectations[0].q, "Generated: Alpha spec");
        assert_eq!(config.expectations[0].a, "yes");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn question_answer_only_uses_resolved_preset_defaults() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default:
    instructions: "Use the preset instructions."
    diff-from: master
    target: diff
    thinking: high
expectations:
  - q: "Does q matching keep preset context?"
    a: "yes"
"#,
        )
        .expect("parse raw check config");

        let config = expand_raw_check_config(
            None,
            Path::new("check.yml"),
            raw,
            None,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand config");

        let expectation = &config.expectations[0];
        assert!(!expectation.question_answer_only);
        assert_eq!(expectation.question_context, "Use the preset instructions.");
        assert_eq!(expectation.diff_from, "master");
        assert_eq!(expectation.target, Some(ExpectationTarget::Diff));
        assert_eq!(expectation.agent.thinking, "high");
    }

    #[test]
    fn expectation_fields_override_preset_defaults() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default:
    q: "Does the preset lose?"
    a: "no"
    instructions: "Preset instructions."
    diff-from: master
    cooldown: 7d
    thinking: medium
expectations:
  - q: "Does the item win?"
    a: "yes"
    instructions: "Item instructions."
    diff-from: HEAD~1
    cooldown: 1d
    thinking: high
"#,
        )
        .expect("parse raw check config");

        let config = expand_raw_check_config(
            None,
            Path::new("check.yml"),
            raw,
            None,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand config");

        let expectation = &config.expectations[0];
        assert_eq!(expectation.q, "Does the item win?");
        assert_eq!(expectation.a, "yes");
        assert_eq!(expectation.question_context, "Item instructions.");
        assert_eq!(expectation.diff_from, "HEAD~1");
        assert_eq!(
            expectation.cooldown,
            Some(CooldownConfig::Compact("1d".to_string()))
        );
        assert_eq!(expectation.agent.thinking, "high");
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
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
