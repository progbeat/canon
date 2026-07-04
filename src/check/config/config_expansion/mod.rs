mod expansion;
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
    use crate::config_types::{
        CheckHookCaseOutcome, CooldownConfig, ExpectationTarget, RawCheckConfig,
    };
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

    // xpec: WH,n7
    #[test]
    fn include_generator_defaults_do_not_reclassify_explicit_child_items() {
        let root = test_root("include-explicit-child-form");
        git(&root, &["init"]);
        fs::create_dir_all(root.join("expects")).unwrap();
        fs::create_dir_all(root.join("expects/specs")).unwrap();
        fs::write(root.join("expects/specs/alpha.md"), "Alpha spec").unwrap();
        fs::write(
            root.join("expects/included.yml"),
            r#"
- q: "Does the child stay explicit?"
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

        assert_eq!(config.expectations.len(), 1);
        assert_eq!(config.expectations[0].q, "Does the child stay explicit?");
        assert_eq!(config.expectations[0].a, "yes");
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

    // xpec: uY
    #[test]
    fn top_level_hooks_expand_shorthand_and_mapping() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default: {}
hooks:
  on-start: "Starting check."
  on-pass:
    - print: "Type pass:"
      input: " "
      cases:
        pass: !ok
        ~: !block "Null key."
        _: !block "Run the blocker fix."
    - exec: ["cargo", "fmt", "--check"]
      cases:
        0: !ok
        _: !block "Format the code."
    - input: "Empty cases:"
      cases: {}
    - "Done."
expectations:
  - q: "Does hook config parse?"
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

        // xpec: uY
        assert_eq!(config.hooks.on_start.len(), 1);
        let on_start = &config.hooks.on_start[0];
        // xpec: uY
        assert_eq!(on_start.print.as_deref(), Some("Starting check."));
        // xpec: uY
        assert_eq!(on_start.input, None);
        // xpec: uY
        assert_eq!(on_start.exec, None);
        // xpec: uY
        assert_eq!(config.hooks.on_pass.len(), 4);
        let on_pass = &config.hooks.on_pass[0];
        // xpec: uY
        assert_eq!(on_pass.print.as_deref(), Some("Type pass:"));
        // xpec: uY
        assert_eq!(on_pass.input.as_deref(), Some(" "));
        // xpec: uY
        assert!(matches!(
            on_pass.cases.get("pass"),
            Some(CheckHookCaseOutcome::Continue)
        ));
        // xpec: uY
        assert!(matches!(
            on_pass.cases.get("null"),
            Some(CheckHookCaseOutcome::Block {
                repair_instruction
            }) if repair_instruction == "Null key."
        ));
        // xpec: uY
        assert!(matches!(
            on_pass.cases.get("_"),
            Some(CheckHookCaseOutcome::Block {
                repair_instruction
            }) if repair_instruction == "Run the blocker fix."
        ));
        let on_pass = &config.hooks.on_pass[1];
        // xpec: uY
        assert_eq!(
            on_pass.exec.as_ref().unwrap(),
            &vec![
                "cargo".to_string(),
                "fmt".to_string(),
                "--check".to_string()
            ]
        );
        // xpec: uY
        assert!(matches!(
            on_pass.cases.get("0"),
            Some(CheckHookCaseOutcome::Continue)
        ));
        let on_pass = &config.hooks.on_pass[2];
        // xpec: uY
        assert_eq!(on_pass.input.as_deref(), Some("Empty cases:"));
        // xpec: uY
        assert!(on_pass.cases.is_empty());
        let on_pass = &config.hooks.on_pass[3];
        // xpec: uY
        assert_eq!(on_pass.print.as_deref(), Some("Done."));
        // xpec: uY
        assert!(on_pass.cases.is_empty());
    }

    // xpec: uY
    #[test]
    fn hook_mapping_validation_rejects_invalid_shapes() {
        let invalid = [
            ("missing action", "hooks:\n  on-start:\n    cases:\n      _: !ok\n"),
            (
                "input with exec",
                "hooks:\n  on-start:\n    input: prompt\n    exec: [tool]\n    cases:\n      _: !ok\n",
            ),
            (
                "cases without input or exec",
                "hooks:\n  on-start:\n    print: prompt\n    cases:\n      _: !ok\n",
            ),
            (
                "empty cases without input or exec",
                "hooks:\n  on-start:\n    print: prompt\n    cases: {}\n",
            ),
            ("input without cases", "hooks:\n  on-start:\n    input: prompt\n"),
        ];
        for (name, hooks) in invalid {
            let raw: RawCheckConfig = serde_saphyr::from_str(&format!(
                r#"
version: 1
presets:
  default: {{}}
{hooks}
expectations:
  - q: "Does hook config parse?"
    a: "yes"
"#
            ))
            .unwrap_or_else(|err| panic!("{name}: failed to parse raw config: {err}"));

            let error = expand_raw_check_config(
                None,
                Path::new("check.yml"),
                raw,
                None,
                CheckConfigSource::Tree(TreeSource::Staged),
            )
            .expect_err("invalid hook shape should fail");

            // xpec: uY
            assert!(
                error.contains("hooks.on-start"),
                "{name}: unexpected error: {error}"
            );
        }
    }

    // xpec: uY
    #[test]
    fn hook_case_outcomes_reject_plain_scalar_tags() {
        serde_saphyr::from_str::<RawCheckConfig>(
            r#"
version: 1
presets:
  default: {}
hooks:
  on-start:
    input: "Continue? "
    cases:
      y: ok
expectations:
  - q: "Does hook config parse?"
    a: "yes"
"#,
        )
        .expect_err("plain scalar hook outcomes must not parse");
    }

    // xpec: uY
    #[test]
    fn hook_list_rejects_nested_hook_lists() {
        let raw = serde_saphyr::from_str::<RawCheckConfig>(
            r#"
version: 1
presets:
  default: {}
hooks:
  on-start:
    - - print: "Nested."
expectations:
  - q: "Does hook config parse?"
    a: "yes"
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
        .expect_err("nested hook list should fail validation");

        // xpec: uY
        assert!(error.contains("hooks.on-start[0]"));
    }

    // xpec: I8
    #[test]
    fn yaml_include_expands_top_level_hooks_from_staged_source() {
        let root = test_root("yaml-include-staged-hooks");
        git(&root, &["init"]);
        fs::create_dir_all(root.join(".canon/hooks")).unwrap();
        fs::write(root.join(".canon/hooks/on-start.yml"), "\"Starting.\"").unwrap();
        fs::write(
            root.join(".canon/check.yml"),
            r#"
version: 1
presets:
  default: {}
hooks:
  on-start: !include hooks/on-start.yml
expectations:
  - q: "Does YAML include parse?"
    a: "yes"
"#,
        )
        .unwrap();
        git(
            &root,
            &["add", ".canon/check.yml", ".canon/hooks/on-start.yml"],
        );
        let mut cache = RepoInspectionCache::new();
        let config = cache
            .load_check_config(&root, Path::new(".canon/check.yml"), &TreeSource::Staged)
            .expect("expand included config");

        // xpec: I8
        assert_eq!(config.hooks.on_start.len(), 1);
        // xpec: I8
        assert_eq!(config.hooks.on_start[0].print.as_deref(), Some("Starting."));
        let _ = fs::remove_dir_all(root);
    }

    // xpec: I8
    #[test]
    fn yaml_include_uses_selected_git_tree_source() {
        let root = test_root("yaml-include-selected-tree");
        git(&root, &["init"]);
        git(&root, &["config", "user.name", "Canon Test"]);
        git(
            &root,
            &["config", "user.email", "canon-test@example.invalid"],
        );
        fs::create_dir_all(root.join(".canon/hooks")).unwrap();
        fs::write(root.join(".canon/hooks/on-start.yml"), "\"From HEAD.\"").unwrap();
        fs::write(
            root.join(".canon/check.yml"),
            r#"
version: 1
presets:
  default: {}
hooks:
  on-start: !include hooks/on-start.yml
expectations:
  - q: "Does YAML include parse?"
    a: "yes"
"#,
        )
        .unwrap();
        git(&root, &["add", ".canon"]);
        git(&root, &["commit", "--quiet", "-m", "base"]);
        fs::write(root.join(".canon/hooks/on-start.yml"), "\"From staged.\"").unwrap();
        git(&root, &["add", ".canon/hooks/on-start.yml"]);
        let selected_tree = TreeSource::resolve(&root, "HEAD", "--tree").unwrap();
        let mut cache = RepoInspectionCache::new();

        let config = cache
            .load_check_config(&root, Path::new(".canon/check.yml"), &selected_tree)
            .expect("expand included config from selected tree");

        // xpec: I8
        assert_eq!(
            config.hooks.on_start[0].print.as_deref(),
            Some("From HEAD.")
        );
        let _ = fs::remove_dir_all(root);
    }

    // xpec: I8
    #[test]
    fn yaml_include_uses_in_place_filesystem_source() {
        let root = test_root("yaml-include-in-place");
        fs::create_dir_all(root.join(".canon/hooks")).unwrap();
        fs::write(
            root.join(".canon/hooks/on-start.yml"),
            "\"From filesystem.\"",
        )
        .unwrap();
        fs::write(
            root.join(".canon/check.yml"),
            r#"
version: 1
presets:
  default: {}
hooks:
  on-start: !include hooks/on-start.yml
expectations:
  - q: "Does YAML include parse?"
    a: "yes"
"#,
        )
        .unwrap();
        let mut cache = RepoInspectionCache::new();

        let config = cache
            .load_in_place_check_config_with_default_agent_preset(
                &root,
                Path::new(".canon/check.yml"),
                None,
            )
            .expect("expand included in-place config");

        // xpec: I8
        assert_eq!(
            config.hooks.on_start[0].print.as_deref(),
            Some("From filesystem.")
        );
        let _ = fs::remove_dir_all(root);
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

    // xpec: WH,n7
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

    // xpec: WH,n7
    #[test]
    fn path_generator_q_template_takes_item_precedence_over_preset_q() {
        let root = test_root("preset-generator-q-template-precedence");
        git(&root, &["init"]);
        fs::create_dir_all(root.join("specs")).unwrap();
        fs::write(root.join("specs/alpha.md"), "Alpha spec").unwrap();
        git(&root, &["add", "specs/alpha.md"]);
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default:
    q: "Does the preset question lose to the path generator item?"
expectations:
  - path: "specs/*.md"
    q_template: "Generated: {{content}}"
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

        assert_eq!(config.expectations.len(), 1);
        assert_eq!(config.expectations[0].q, "Generated: Alpha spec");
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
    instructions: " Item instructions. "
    diff-from: " HEAD~1 "
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
        assert_eq!(expectation.question_context, " Item instructions. ");
        assert_eq!(expectation.diff_from, " HEAD~1 ");
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
