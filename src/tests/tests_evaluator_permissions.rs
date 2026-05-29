use super::*;

#[test]
fn evaluator_permissions_allow_only_materialized_working_tree_without_scope_filters() {
    let agent = AgentConfig {
        models: Vec::new(),
        thinking: "low".to_string(),
        instructions: Some("Answer from files only.".to_string()),
        ignore: vec!["target/**".to_string()],
        plugins: Vec::new(),
    };
    let session_root = Path::new("/tmp/canon-check-snapshot");
    let config =
        evaluator_thread_config(&agent, &full_scope(), None, &agent.thinking, session_root);
    let filesystem = config["permissions"]["canon_check"]["filesystem"]
        .as_object()
        .unwrap();
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"][":root"],
        "deny"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"][":minimal"],
        "read"
    );
    assert_eq!(filesystem[&session_path(session_root, ".")], "read");
    assert_eq!(filesystem[&session_glob(session_root, "**")], "read");
    assert!(filesystem.get(":workspace_roots").is_none());
    assert!(filesystem.get(".canon").is_none());
    assert!(filesystem.get(".canon/**").is_none());
    assert!(filesystem.get("target").is_none());
    assert!(filesystem.get("target/**").is_none());
    assert!(filesystem
        .get(&session_path(session_root, ".canon"))
        .is_none());
    assert!(filesystem
        .get(&session_glob(session_root, ".canon/**"))
        .is_none());
    assert!(filesystem
        .get(&session_path(session_root, "target"))
        .is_none());
    assert!(filesystem
        .get(&session_glob(session_root, "target/**"))
        .is_none());
    assert_eq!(filesystem["/etc/**"], "read");
    assert_eq!(filesystem["/private/etc/**"], "read");
    assert_eq!(filesystem["/usr/share/**"], "read");
    assert_eq!(filesystem["~"], "read");
    assert_eq!(filesystem["~/.zshenv"], "read");
    assert!(filesystem.get("~/**").is_none());
    assert_eq!(config["model_reasoning_effort"], "low");
    assert!(config["permissions"]["canon_check"]["filesystem"]
        .get("~/.codex/tmp/**")
        .is_none());
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"][":tmpdir"],
        "deny"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"][":slash_tmp"],
        "deny"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["/tmp"],
        "deny"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["/tmp/**"],
        "deny"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["/private/tmp"],
        "deny"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["/private/tmp/**"],
        "deny"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["~/.codex/sessions"],
        "deny"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["~/.codex/sessions/**"],
        "deny"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["~/.codex/memories"],
        "deny"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["~/.codex/memories/**"],
        "deny"
    );
    assert!(config["permissions"]["canon_check"]["filesystem"]
        .as_object()
        .unwrap()
        .values()
        .all(|permission| permission != "write"));
    assert_eq!(config["history"]["persistence"], "none");
    assert_eq!(config["include_environment_context"], json!(false));
    assert_eq!(config["include_permissions_instructions"], json!(false));
    assert_eq!(config["include_apps_instructions"], json!(false));
    assert_eq!(config["include_apply_patch_tool"], json!(false));
    assert_eq!(
        config["experimental_use_freeform_apply_patch"],
        json!(false)
    );
    assert_eq!(config["features"]["apply_patch_freeform"], json!(false));
    assert_eq!(config["features"]["apps"], json!(false));
    assert_eq!(config["features"]["tool_search"], json!(false));
    assert_eq!(config["features"]["unified_exec"], json!(false));
    assert!(config["features"].get("shell_tool").is_none());
    assert_eq!(config["project_doc_max_bytes"], json!(0));
    assert!(config.get("plugins").is_none());
}

#[test]
fn no_sandbox_thread_config_keeps_snapshot_permission_profile() {
    let agent = AgentConfig {
        models: Vec::new(),
        thinking: "low".to_string(),
        instructions: Some("Answer from files only.".to_string()),
        ignore: vec!["target/**".to_string()],
        plugins: Vec::new(),
    };
    let config = evaluator_thread_config_with_no_sandbox(
        &agent,
        &full_scope(),
        None,
        &agent.thinking,
        Path::new("/tmp/canon-check-snapshot"),
        true,
    );

    assert_eq!(config["sandbox_mode"], "danger-full-access");
    assert_eq!(config["default_permissions"], "canon_check");
    let filesystem = config["permissions"]["canon_check"]["filesystem"]
        .as_object()
        .unwrap();
    assert_eq!(filesystem[":root"], "deny");
    assert_eq!(
        filesystem[&session_path(Path::new("/tmp/canon-check-snapshot"), ".")],
        "read"
    );
    assert_eq!(
        filesystem[&session_glob(Path::new("/tmp/canon-check-snapshot"), "**")],
        "read"
    );
    assert_eq!(config["history"]["persistence"], "none");
    assert_eq!(config["model_reasoning_effort"], "low");
    assert_eq!(config["include_environment_context"], json!(false));
    assert_eq!(config["include_permissions_instructions"], json!(false));
    assert_eq!(config["include_apps_instructions"], json!(false));
    assert_eq!(config["features"]["tool_search"], json!(false));
    assert_eq!(config["project_doc_max_bytes"], json!(0));
}

#[test]
fn evaluator_working_tree_permissions_do_not_encode_restricted_scope() {
    let agent = AgentConfig {
        models: Vec::new(),
        thinking: "low".to_string(),
        instructions: Some("Answer from files only.".to_string()),
        ignore: vec!["target/**".to_string()],
        plugins: Vec::new(),
    };
    let session_root = Path::new("/tmp/canon-check-snapshot/scopes/0");
    let config = evaluator_thread_config(
        &agent,
        &["src/bin/main.rs".to_string()],
        None,
        &agent.thinking,
        session_root,
    );
    let filesystem = config["permissions"]["canon_check"]["filesystem"]
        .as_object()
        .unwrap();

    assert_eq!(filesystem[&session_path(session_root, ".")], "read");
    assert_eq!(filesystem[&session_glob(session_root, "**")], "read");
    assert!(filesystem.get(&session_path(session_root, "src")).is_none());
    assert!(filesystem
        .get(&session_path(session_root, "src/bin/main.rs"))
        .is_none());
    assert!(filesystem.get("src").is_none());
    assert!(filesystem.get("src/**").is_none());
    assert!(filesystem.get("target/**").is_none());

    let working_tree_permissions = evaluator_working_tree_permissions(session_root);
    assert_eq!(
        working_tree_permissions[&session_path(session_root, ".")],
        "read"
    );
    assert_eq!(
        working_tree_permissions[&session_glob(session_root, "**")],
        "read"
    );
}

fn session_path(root: &Path, path: &str) -> String {
    if path == "." {
        root.display().to_string()
    } else {
        root.join(path).display().to_string()
    }
}

fn session_glob(root: &Path, pattern: &str) -> String {
    root.join(pattern).display().to_string()
}

#[test]
fn evaluator_model_is_configured_when_present() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let thread_config = evaluator_thread_config(
        &config.agent,
        &full_scope(),
        None,
        &config.agent.thinking,
        Path::new("/tmp/canon-check-snapshot"),
    );
    assert_eq!(thread_config["model"], "gpt-5.4-mini");
    let fallback_config = evaluator_thread_config(
        &config.agent,
        &full_scope(),
        Some("gpt-5.3-codex-spark"),
        &config.agent.thinking,
        Path::new("/tmp/canon-check-snapshot"),
    );
    assert_eq!(fallback_config["model"], "gpt-5.3-codex-spark");
}

#[test]
fn evaluator_plugin_list_is_explicitly_configured() {
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins:
    - "canon@codex-plugins"
expectations:
  - q: "Question?"
    a: "yes"
"#,
    )
    .unwrap();
    assert!(check_config_loads_plugins(&config));
    let thread_config = evaluator_thread_config(
        &config.agent,
        &full_scope(),
        None,
        &config.agent.thinking,
        Path::new("/tmp/canon-check-snapshot"),
    );
    assert_eq!(
        thread_config["plugins"]["canon@codex-plugins"]["enabled"],
        json!(true)
    );
}

#[test]
fn plugin_loading_uses_default_agent_plugins_directly() {
    let mut config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins:
    - "canon@codex-plugins"
expectations:
  - q: "Question?"
    a: "yes"
"#,
    )
    .unwrap();
    config.expectations[0].agent.plugins.clear();

    assert!(check_config_loads_plugins(&config));
}

#[test]
fn app_server_starts_with_plugins_disabled_by_default() {
    let root = git_project("app-server-args-default");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let disabled = app_server_args(&root, false, &config.agent).unwrap();
    assert_eq!(&disabled[..3], ["app-server", "--disable", "plugins"]);
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["--disable", "apps"]));
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["--disable", "tool_search"]));
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["--disable", "unified_exec"]));
    assert!(!disabled
        .windows(2)
        .any(|pair| pair == ["--disable", "shell_tool"]));
    assert_eq!(&disabled[disabled.len() - 2..], ["--listen", "stdio://"]);
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "default_permissions=\"canon_check\""]));
    assert!(!disabled
        .windows(2)
        .any(|pair| pair[0] == "-c" && pair[1].starts_with("model=")));
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "model_reasoning_effort=\"medium\""]));
    let model_catalog_arg = disabled
        .windows(2)
        .find_map(|pair| {
            (pair[0] == "-c" && pair[1].starts_with("model_catalog_json="))
                .then_some(pair[1].as_str())
        })
        .unwrap();
    assert!(model_catalog_arg.starts_with("model_catalog_json=\""));
    let model_catalog_path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join("canon-evaluator-model-catalogs")
        .join(format!("{}.json", process::id()));
    assert_eq!(
        model_catalog_arg,
        format!(
            "model_catalog_json={}",
            toml_string(&model_catalog_path.to_string_lossy())
        )
    );
    let model_catalog: Value =
        serde_json::from_str(&fs::read_to_string(&model_catalog_path).unwrap()).unwrap();
    let models = model_catalog["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0]["slug"], "gpt-5.4-mini");
    assert_eq!(models[1]["slug"], "gpt-5.3-codex-spark");
    assert_eq!(models[0]["apply_patch_tool_type"], Value::Null);
    assert_eq!(models[1]["apply_patch_tool_type"], Value::Null);
    assert_eq!(models[0]["base_instructions"], "");
    assert!(models[0].get("model_messages").is_none());
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "include_environment_context=false"]));
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "include_permissions_instructions=false"]));
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "include_apps_instructions=false"]));
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "include_apply_patch_tool=false"]));
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "experimental_use_freeform_apply_patch=false"]));
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "features.apps=false"]));
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "features.tool_search=false"]));
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "features.unified_exec=false"]));
    assert!(!disabled
        .windows(2)
        .any(|pair| pair == ["-c", "features.shell_tool=false"]));
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "features.apply_patch_freeform=false"]));
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "project_doc_max_bytes=0"]));
    let filesystem_arg = disabled
        .windows(2)
        .find_map(|pair| {
            (pair[0] == "-c" && pair[1].starts_with("permissions.canon_check.filesystem="))
                .then_some(pair[1].as_str())
        })
        .unwrap();
    assert!(!filesystem_arg.contains(r#"":workspace_roots""#));
    assert!(!filesystem_arg.contains(r#"".canon/**"="deny""#));
    assert!(!filesystem_arg.contains(r#""target"="deny""#));
    assert!(!filesystem_arg.contains(r#""target/**"="deny""#));
    assert!(filesystem_arg.contains(r#"":root"="read""#));
    assert!(filesystem_arg.contains(r#"":minimal"="read""#));
    assert!(filesystem_arg.contains(r#"":tmpdir"="deny""#));
    assert!(filesystem_arg.contains(r#"":slash_tmp"="deny""#));
    assert!(filesystem_arg.contains(r#""/tmp"="deny""#));
    assert!(filesystem_arg.contains(r#""/tmp/**"="deny""#));
    assert!(filesystem_arg.contains(r#""/private/tmp"="deny""#));
    assert!(filesystem_arg.contains(r#""/private/tmp/**"="deny""#));
    assert!(!filesystem_arg.contains(r#""~/.codex/tmp/**""#));
    assert!(filesystem_arg.contains(r#""glob_scan_max_depth"=32"#));
    assert!(filesystem_arg.contains(r#""~/.codex/sessions"="deny""#));
    assert!(filesystem_arg.contains(r#""~/.codex/sessions/**"="deny""#));
    assert!(filesystem_arg.contains(r#""~/.codex/memories"="deny""#));
    assert!(filesystem_arg.contains(r#""~/.codex/memories/**"="deny""#));
    assert!(!filesystem_arg.contains(r#""write""#));
    assert!(!filesystem_arg.contains(r#""."="read""#));
    assert!(disabled
        .windows(2)
        .any(|pair| { pair == ["-c", "thread_reuse.carryover_token_target=[10000,30000]",] }));

    let enabled = app_server_args(&root, true, &config.agent).unwrap();
    assert_eq!(enabled.first().map(String::as_str), Some("app-server"));
    assert!(!enabled
        .windows(2)
        .any(|pair| pair == ["--disable", "plugins"]));
    assert!(enabled.windows(2).any(|pair| pair == ["--disable", "apps"]));
    assert!(!enabled
        .windows(2)
        .any(|pair| pair == ["--disable", "shell_tool"]));
    assert_eq!(&enabled[enabled.len() - 2..], ["--listen", "stdio://"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn no_sandbox_app_server_args_keep_canon_permission_profile() {
    let root = git_project("app-server-args-no-sandbox");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let args = app_server_startup_config_args_with_no_sandbox(&root, &config.agent, true).unwrap();

    assert!(args
        .windows(2)
        .any(|pair| pair == ["-c", "sandbox_mode=\"danger-full-access\""]));
    assert!(args
        .windows(2)
        .any(|pair| pair == ["-c", "default_permissions=\"canon_check\""]));
    assert!(args
        .windows(2)
        .any(|pair| { pair[0] == "-c" && pair[1].starts_with("permissions.canon_check.") }));
    assert!(args
        .windows(2)
        .any(|pair| pair == ["-c", "include_environment_context=false"]));
    assert!(args
        .windows(2)
        .any(|pair| pair == ["-c", "features.tool_search=false"]));
    assert!(args
        .windows(2)
        .any(|pair| { pair == ["-c", "thread_reuse.carryover_token_target=[10000,30000]",] }));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn evaluator_model_catalog_json_removes_model_prompt_and_patch_tool() {
    let catalog = evaluator_model_catalog_json(&[
        "gpt-5.4-mini".to_string(),
        "gpt-5.3-codex-spark".to_string(),
    ])
    .unwrap();
    let value: Value = serde_json::from_str(&catalog).unwrap();
    let models = value["models"].as_array().unwrap();

    assert_eq!(models[0]["slug"], "gpt-5.4-mini");
    assert_eq!(models[1]["slug"], "gpt-5.3-codex-spark");
    for model in models {
        assert_eq!(model["apply_patch_tool_type"], Value::Null);
        assert_eq!(model["base_instructions"], "");
        assert!(model.get("model_messages").is_none());
    }
}

#[test]
fn evaluator_codex_home_preserves_auth_without_skills_or_plugins() {
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CODEX_HOME", "CODEX_SANDBOX"]);
    let source_home = TestDir::new("codex-home-source");
    let user_skill = source_home.path().join("skills/team/SKILL.md");
    let plugin_skill = source_home
        .path()
        .join("plugins/cache/example-plugin/skills/audit/SKILL.md");
    ensure_dir(user_skill.parent().unwrap()).unwrap();
    ensure_dir(plugin_skill.parent().unwrap()).unwrap();
    fs::write(&user_skill, "name: team\n").unwrap();
    fs::write(&plugin_skill, "name: audit\n").unwrap();
    fs::write(source_home.path().join("auth.json"), "{}\n").unwrap();
    env_snapshot.set("CODEX_HOME", source_home.path());
    env_snapshot.remove("CODEX_SANDBOX");

    let root = git_project("app-server-empty-codex-home");
    let evaluator_home = prepare_evaluator_codex_home(&root).unwrap();
    let canonical_root = root.canonicalize().unwrap();

    assert_eq!(
        evaluator_home,
        resolve_git_path(&canonical_root, "canon/evaluator-codex-home/.codex").unwrap()
    );
    assert!(evaluator_home.starts_with(resolve_git_path(&canonical_root, "canon").unwrap()));
    let auth_path = evaluator_home.join("auth.json");
    assert!(auth_path.exists());
    #[cfg(unix)]
    {
        assert!(fs::symlink_metadata(&auth_path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_link(&auth_path).unwrap(),
            source_home.path().join("auth.json")
        );
    }
    #[cfg(not(unix))]
    assert!(!fs::symlink_metadata(&auth_path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(evaluator_home.join("skills").is_dir());
    assert!(evaluator_home.join("plugins").is_dir());
    assert!(evaluator_home
        .join("skills/.system/.codex-system-skills.marker")
        .is_file());
    assert!(!evaluator_home.join("skills/team/SKILL.md").exists());
    assert!(!evaluator_home
        .join("plugins/cache/example-plugin/skills/audit/SKILL.md")
        .exists());
    let _ = fs::remove_dir_all(evaluator_home);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn evaluator_codex_home_symlinks_auth_inside_codex_sandbox() {
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CODEX_HOME", "CODEX_SANDBOX"]);
    let source_home = TestDir::new("codex-home-source-sandbox");
    fs::write(source_home.path().join("auth.json"), "{}\n").unwrap();
    env_snapshot.set("CODEX_HOME", source_home.path());
    env_snapshot.set("CODEX_SANDBOX", "seatbelt");

    let root = git_project("app-server-copied-codex-home");
    let evaluator_home = prepare_evaluator_codex_home(&root).unwrap();

    let auth_path = evaluator_home.join("auth.json");
    assert!(auth_path.exists());
    #[cfg(unix)]
    {
        assert!(fs::symlink_metadata(&auth_path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_link(&auth_path).unwrap(),
            source_home.path().join("auth.json")
        );
    }
    #[cfg(not(unix))]
    assert!(!fs::symlink_metadata(&auth_path)
        .unwrap()
        .file_type()
        .is_symlink());
    let _ = fs::remove_dir_all(evaluator_home);
    let _ = fs::remove_dir_all(root);
}

#[test]
#[cfg(unix)]
fn evaluator_codex_home_skips_auth_mirror_when_source_is_same_home_alias() {
    use std::os::unix::fs::symlink;

    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CODEX_HOME", "CODEX_SANDBOX"]);
    env_snapshot.remove("CODEX_SANDBOX");
    let root = git_project("app-server-same-codex-home-alias");
    let canonical_root = root.canonicalize().unwrap();
    let evaluator_home =
        resolve_git_path(&canonical_root, "canon/evaluator-codex-home/.codex").unwrap();
    fs::create_dir_all(&evaluator_home).unwrap();
    let auth_path = evaluator_home.join("auth.json");
    fs::write(&auth_path, "{}\n").unwrap();
    let alias_root = TestDir::new("codex-home-same-target-alias");
    let alias_path = alias_root.path().join("alias");
    symlink(&evaluator_home, &alias_path).unwrap();
    env_snapshot.set("CODEX_HOME", &alias_path);

    let prepared_home = prepare_evaluator_codex_home(&root).unwrap();

    assert_eq!(prepared_home, evaluator_home);
    assert_eq!(fs::read_to_string(&auth_path).unwrap(), "{}\n");
    assert!(!fs::symlink_metadata(&auth_path)
        .unwrap()
        .file_type()
        .is_symlink());
    let _ = fs::remove_dir_all(evaluator_home);
    let _ = fs::remove_dir_all(root);
}

#[test]
#[cfg(unix)]
fn evaluator_codex_home_rejects_symlinked_state_parent() {
    use std::os::unix::fs::symlink;

    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CODEX_HOME"]);
    env_snapshot.remove("CODEX_HOME");
    let root = git_project("app-server-symlinked-temp-canon");
    let state_root = resolve_git_path(&root, "canon").unwrap();
    fs::create_dir_all(&state_root).unwrap();
    let target_root = TestDir::new("codex-home-state-target");
    symlink(target_root.path(), state_root.join("evaluator-codex-home")).unwrap();

    let err = prepare_evaluator_codex_home(&root).unwrap_err();

    assert!(err.contains("refusing to use symlink"), "{err}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn app_server_environment_does_not_inherit_parent_secrets() {
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&[
        "CANON_TEST_SECRET",
        "CODEX_HOME",
        "CODEX_THREAD_ID",
        "HOME",
        "PATH",
    ]);
    let expected_path = env::var_os("PATH").map(|path| path.to_string_lossy().into_owned());
    env_snapshot.set("CANON_TEST_SECRET", "secret-token");
    env_snapshot.set("CODEX_HOME", "/tmp/source-codex-home");
    env_snapshot.set("CODEX_THREAD_ID", "parent-thread");
    env_snapshot.set("HOME", "/tmp/real-home");
    let isolated_home = Path::new("/tmp/canon/.codex");
    let mut command = Command::new("codex");

    configure_app_server_environment(&mut command, Some(isolated_home)).unwrap();

    let envs = command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert!(!envs.contains_key("CANON_TEST_SECRET"));
    assert!(!envs.contains_key("CODEX_THREAD_ID"));
    assert_eq!(
        envs.get("CODEX_HOME").and_then(|value| value.as_deref()),
        Some("/tmp/canon/.codex")
    );
    assert_eq!(
        envs.get("HOME").and_then(|value| value.as_deref()),
        Some("/tmp/canon")
    );
    assert_eq!(
        envs.get("PATH").and_then(|value| value.as_deref()),
        expected_path.as_deref()
    );
    assert!(envs.contains_key("TMPDIR"));
}

#[test]
fn app_server_startup_config_escapes_toml_control_characters() {
    let filesystem_arg = app_server_startup_filesystem_arg();

    assert!(!filesystem_arg.contains("quoted"));
    assert!(!filesystem_arg.contains("control"));
    assert!(!filesystem_arg.contains("delete"));
    assert!(!filesystem_arg.contains('\u{0007}'));
    assert!(!filesystem_arg.contains('\u{007f}'));
}

#[test]
fn thread_reuse_config_reads_git_carryover_token_target() {
    let root = git_project("thread-reuse-config");
    let output = Command::new("git")
        .args([
            "config",
            "canon.threadReuse.carryoverTokenTarget",
            "12000,24000",
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success());

    let config = thread_reuse_config(&root).unwrap();

    assert_eq!(config.carryover_token_target.min, 12_000);
    assert_eq!(config.carryover_token_target.max, 24_000);
    assert_eq!(
        thread_reuse_carryover_token_target_arg(&config),
        "thread_reuse.carryover_token_target=[12000,24000]"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn thread_reuse_config_defaults_and_validates_carryover_token_target() {
    let root = git_project("thread-reuse-config-default");
    assert_eq!(
        thread_reuse_config(&root).unwrap(),
        DEFAULT_THREAD_REUSE_CONFIG
    );

    assert!(parse_carryover_token_target("30000,10000")
        .unwrap_err()
        .contains("MIN"));
    assert!(parse_carryover_token_target("10000")
        .unwrap_err()
        .contains("MIN,MAX"));
    assert!(parse_carryover_token_target("0,10000")
        .unwrap_err()
        .contains("greater than zero"));
    let _ = fs::remove_dir_all(root);
}
