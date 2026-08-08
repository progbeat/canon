use super::*;
use serde_json::json;

fn context<'a>(
    agent: &'a AgentConfig,
    dynamic_tools: &'a [Value],
) -> AppServerThreadStartContext<'a> {
    static HOST_ISOLATION: std::sync::LazyLock<EvaluatorHostIsolation> =
        std::sync::LazyLock::new(EvaluatorHostIsolation::in_place);
    AppServerThreadStartContext {
        cwd: Path::new("/cwd"),
        template_artifact_directory: Path::new("/artifacts"),
        host_isolation: &HOST_ISOLATION,
        rendered_base_text: "base",
        rendered_developer_text: "developer",
        agent,
        model: None,
        thinking: "medium",
        app_server_state_root: Some(Path::new("/state")),
        process_isolation: EvaluatorProcessIsolation::CanonManaged,
        dynamic_tools,
        codex_executable: Path::new("/runtime/codex"),
    }
}

#[test] // xpec: hQ
fn canon_managed_thread_serializes_the_named_permission_contract() {
    let mut memo = InvocationThreadStartMemo::default();
    let params = memo.resolve(context(&AgentConfig::default(), &[])).unwrap();

    // [hQ] These keys and values are the public Codex app-server request
    // contract selected by canon-managed evaluator isolation.
    assert_eq!(params["permissions"], json!("canon_check"));
    assert!(params.get("sandbox").is_none());
    assert_eq!(
        params["config"]["default_permissions"],
        json!("canon_check")
    );
}

#[test] // xpec: hQ
fn externally_isolated_thread_serializes_the_legacy_permission_contract() {
    let agent = AgentConfig::default();
    let mut external_context = context(&agent, &[]);
    external_context.process_isolation = EvaluatorProcessIsolation::ExternallyManaged;
    let params = InvocationThreadStartMemo::default()
        .resolve(external_context)
        .unwrap();

    // [hQ] These assertions cover the alternative public app-server
    // request contract, not InvocationThreadStartMemo's representation.
    assert!(params.get("permissions").is_none());
    assert_eq!(params["sandbox"], json!("danger-full-access"));
    assert_eq!(
        params["config"]["sandbox_mode"],
        json!("danger-full-access")
    );
    assert!(params["config"].get("default_permissions").is_none());
    assert!(params["config"].get("permissions.canon_check").is_none());
}

#[test] // xpec: d,gN,bP
fn thread_start_params_cover_direct_and_evaluator_owned_values() {
    let agent = AgentConfig {
        models: vec!["model".to_string()],
        plugins: vec!["plugin".to_string()],
        ..AgentConfig::default()
    };
    let changed_model_agent = AgentConfig {
        models: vec!["other-model".to_string()],
        plugins: agent.plugins.clone(),
        ..AgentConfig::default()
    };
    let changed_plugin_agent = AgentConfig {
        models: agent.models.clone(),
        plugins: vec!["other-plugin".to_string()],
        ..AgentConfig::default()
    };
    let dynamic_tools = vec![json!({"name": "tool"})];
    let mut memo = InvocationThreadStartMemo::default();
    let baseline = memo.resolve(context(&agent, &dynamic_tools)).unwrap();
    let changed_agent_model = memo
        .resolve(context(&changed_model_agent, &dynamic_tools))
        .unwrap();
    let changed_agent_plugin = memo
        .resolve(context(&changed_plugin_agent, &dynamic_tools))
        .unwrap();
    let mut changed_context = context(&agent, &dynamic_tools);
    changed_context.model = Some("explicit-model");
    let changed_explicit_model = memo.resolve(changed_context).unwrap();
    let mut changed_context = context(&agent, &dynamic_tools);
    changed_context.thinking = "high";
    let changed_thinking = memo.resolve(changed_context).unwrap();
    let mut changed_context = context(&agent, &dynamic_tools);
    changed_context.app_server_state_root = Some(Path::new("/other-state"));
    let changed_state_root = memo.resolve(changed_context).unwrap();
    let mut changed_context = context(&agent, &dynamic_tools);
    changed_context.cwd = Path::new("/other-cwd");
    let changed_cwd = memo.resolve(changed_context).unwrap();
    let mut changed_context = context(&agent, &dynamic_tools);
    changed_context.rendered_base_text = "other-base";
    let changed_base = memo.resolve(changed_context).unwrap();
    let mut changed_context = context(&agent, &dynamic_tools);
    changed_context.rendered_developer_text = "other-developer";
    let changed_developer = memo.resolve(changed_context).unwrap();
    let mut changed_context = context(&agent, &dynamic_tools);
    changed_context.process_isolation = EvaluatorProcessIsolation::ExternallyManaged;
    let changed_isolation = memo.resolve(changed_context).unwrap();
    let changed_tools = memo.resolve(context(&agent, &[])).unwrap();
    let mut changed_context = context(&agent, &dynamic_tools);
    changed_context.template_artifact_directory = Path::new("/other-artifacts");
    let changed_evaluator_values = memo.resolve(changed_context).unwrap();
    let mut changed_context = context(&agent, &dynamic_tools);
    changed_context.codex_executable = Path::new("/other-runtime/codex");
    let changed_executable = memo.resolve(changed_context).unwrap();

    for changed in [
        changed_agent_model,
        changed_agent_plugin,
        changed_explicit_model,
        changed_thinking,
        changed_cwd,
        changed_base,
        changed_developer,
        changed_isolation,
        changed_tools,
        changed_evaluator_values,
        changed_executable,
    ] {
        assert_ne!(baseline, changed);
    }
    assert_eq!(baseline, changed_state_root);
}

#[test] // xpec: d,gN
fn different_wire_text_preserves_runtime_config() {
    let mut memo = InvocationThreadStartMemo::default();
    let agent = AgentConfig::default();
    let first = memo.resolve(context(&agent, &[])).unwrap();
    let mut changed_context = context(&agent, &[]);
    changed_context.rendered_base_text = "different base";
    let second = memo.resolve(changed_context).unwrap();

    assert_eq!(first["baseInstructions"], json!("base"));
    assert_eq!(second["baseInstructions"], json!("different base"));
    assert_eq!(first["config"], second["config"]);
}
