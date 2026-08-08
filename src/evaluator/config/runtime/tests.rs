use super::*;
use serde_json::json;

#[test] // xpec: A8,bP
fn startup_config_uses_a_read_only_host_root() {
    let agent = AgentConfig::default();
    let config = evaluator_startup_config_settings(&agent, Path::new("/runtime/codex"))
        .to_json_value()
        .unwrap();
    let profile_key = format!("permissions.{EVALUATOR_PERMISSION_PROFILE}");

    assert_eq!(
        config[profile_key]["filesystem"][":root"],
        json!(FILESYSTEM_READ)
    );
    assert_eq!(config["history.persistence"], json!("none"));
    assert!(config.get("permissions").is_none());
}

#[test] // xpec: bP,hQ
fn externally_managed_config_uses_only_the_legacy_sandbox_selection() {
    let agent = AgentConfig::default();
    let config = evaluator_startup_config_settings(&agent, Path::new("/runtime/codex"))
        .with_process_isolation(EvaluatorProcessIsolation::ExternallyManaged)
        .to_json_value()
        .unwrap();

    assert_eq!(config["sandbox_mode"], json!("danger-full-access"));
    assert!(config.get("default_permissions").is_none());
    assert!(config.get("permissions.canon_check").is_none());
}

#[test] // xpec: bP,hQ
fn startup_permission_profile_reads_the_codex_executable() {
    let agent = AgentConfig::default();
    let config = evaluator_startup_config_settings(
        &agent,
        Path::new("/opt/codex/releases/current/bin/codex"),
    )
    .to_json_value()
    .unwrap();
    let profile_key = format!("permissions.{EVALUATOR_PERMISSION_PROFILE}");

    assert_eq!(config[profile_key.as_str()]["extends"], json!(":read-only"));
    assert_eq!(
        config[profile_key.as_str()]["filesystem"]["/opt/codex/releases/current/bin/codex"],
        json!("read")
    );
}
