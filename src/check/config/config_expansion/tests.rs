use super::{expand_raw_check_config, CheckConfigSource};
use crate::config_types::{CooldownConfig, RawCheckConfig};
use crate::git::TreeSource;
use crate::repo_inspection::RepoInspectionCache;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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
    assert_eq!(config.presets.get("default"), Some(&config.agent));
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
