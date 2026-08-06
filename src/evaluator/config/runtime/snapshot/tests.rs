use super::super::super::permissions::evaluator_baseline_permissions;
use super::*;
use serde_json::json;

fn context<'a>(
    agent: &'a AgentConfig,
    host_isolation: &'a EvaluatorHostIsolation,
    process_isolation: EvaluatorProcessIsolation,
) -> EvaluatorRuntimeConfigContext<'a> {
    EvaluatorRuntimeConfigContext {
        agent,
        model: None,
        thinking: "medium",
        app_server_state_root: None,
        session_root: Path::new("/sandbox/2"),
        template_artifact_directory: Path::new("/artifacts"),
        host_isolation,
        process_isolation,
        codex_executable: Path::new("/runtime/codex"),
    }
}

#[test] // xpec: A8,vr,hQ,KD,bP,DB
fn codex_app_server_profile_reads_only_declared_evaluator_inputs() {
    let agent = AgentConfig::default();
    let host_isolation = EvaluatorHostIsolation::from_protected_roots([
        PathBuf::from("/host/home"),
        PathBuf::from("/host/home/project"),
    ]);
    let mut primary_context = context(
        &agent,
        &host_isolation,
        EvaluatorProcessIsolation::CanonManaged,
    );
    primary_context.app_server_state_root = Some(Path::new("/sandbox/2/.git/canon"));
    let snapshot = EvaluatorRuntimeConfigSnapshot::capture(primary_context);
    let config = snapshot.to_json_value().unwrap();
    let mut expected_filesystem =
        evaluator_baseline_permissions(Path::new("/runtime/codex")).unwrap();
    for (path, permission) in [
        (":minimal", "read"),
        (":root", "read"),
        ("/artifacts", "read"),
        ("/host/home", "deny"),
        ("/sandbox/2", "read"),
        ("/sandbox/2/.git/canon", "deny"),
    ] {
        assert!(expected_filesystem
            .insert(path.to_string(), permission.to_string())
            .is_none());
    }

    // The filesystem object is the app-server permission contract. Exact
    // comparison protects the evaluator access boundary, not an internal
    // serialization detail.
    assert_eq!(
        config["permissions.canon_check"]["filesystem"],
        serde_json::to_value(expected_filesystem).unwrap()
    );

    let mut external_state_context = context(
        &agent,
        &host_isolation,
        EvaluatorProcessIsolation::CanonManaged,
    );
    external_state_context.app_server_state_root = Some(Path::new("/state"));
    let external_state_snapshot = EvaluatorRuntimeConfigSnapshot::capture(external_state_context);
    let external_state_config = external_state_snapshot.to_json_value().unwrap();
    assert!(
        external_state_config["permissions.canon_check"]["filesystem"]
            .get("/state")
            .is_none()
    );
}

#[cfg(unix)]
#[test] // xpec: A8,bP
fn runtime_profile_rejects_non_utf8_session_root() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let agent = AgentConfig::default();
    let host_isolation =
        EvaluatorHostIsolation::from_protected_roots([PathBuf::from("/host/home")]);
    let mut non_utf8_context = context(
        &agent,
        &host_isolation,
        EvaluatorProcessIsolation::CanonManaged,
    );
    non_utf8_context.session_root = Path::new(OsStr::from_bytes(b"/tmp/canon-\xff"));
    let snapshot = EvaluatorRuntimeConfigSnapshot::capture(non_utf8_context);

    assert!(snapshot.to_json_value().is_err());
}

#[cfg(unix)]
#[test] // xpec: gO
fn serialized_profile_preserves_filesystem_root() {
    let agent = AgentConfig::default();
    let host_isolation =
        EvaluatorHostIsolation::from_protected_roots([PathBuf::from("/host/home")]);
    let mut root_artifact_context = context(
        &agent,
        &host_isolation,
        EvaluatorProcessIsolation::CanonManaged,
    );
    root_artifact_context.session_root = Path::new("/sandbox");
    root_artifact_context.template_artifact_directory = Path::new("/");
    let artifact_snapshot = EvaluatorRuntimeConfigSnapshot::capture(root_artifact_context);
    let artifact_config = artifact_snapshot.to_json_value().unwrap();
    let artifact_filesystem = &artifact_config["permissions.canon_check"]["filesystem"];

    assert_eq!(artifact_filesystem["/"], json!("read"));
    assert!(artifact_filesystem.get("").is_none());
}

#[test] // xpec: bP,hQ
fn externally_managed_snapshot_does_not_mix_permission_interfaces() {
    let agent = AgentConfig::default();
    let host_isolation = EvaluatorHostIsolation::from_protected_roots([PathBuf::from("/sandbox")]);
    let mut external_context = context(
        &agent,
        &host_isolation,
        EvaluatorProcessIsolation::ExternallyManaged,
    );
    external_context.app_server_state_root = Some(Path::new("/state"));
    let snapshot = EvaluatorRuntimeConfigSnapshot::capture(external_context);
    let config = snapshot.to_json_value().unwrap();

    assert_eq!(config["sandbox_mode"], json!("danger-full-access"));
    assert!(config.get("default_permissions").is_none());
    assert!(config.get("permissions.canon_check").is_none());
}

#[test] // xpec: A8,KD,bP
fn canon_managed_snapshot_rejects_runtime_under_protected_host_root() {
    let agent = AgentConfig::default();
    let host_isolation = EvaluatorHostIsolation::from_protected_roots([PathBuf::from("/sandbox")]);
    let snapshot = EvaluatorRuntimeConfigSnapshot::capture(context(
        &agent,
        &host_isolation,
        EvaluatorProcessIsolation::CanonManaged,
    ));

    assert!(matches!(
        snapshot.to_json_value(),
        Err(super::super::super::EvaluatorConfigError::
            RuntimeInputInsideProtectedHostRoot {
                input,
                protected_root,
            }) if input == Path::new("/sandbox/2")
                && protected_root == Path::new("/sandbox")
    ));
}
