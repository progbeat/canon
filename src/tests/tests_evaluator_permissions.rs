use super::*;

#[test]
fn evaluator_permissions_always_deny_canon_and_agent_ignores() {
    let agent = AgentConfig {
        model: ModelConfig::default(),
        thinking: "low".to_string(),
        instructions: Some("Answer from files only.".to_string()),
        ignore: vec!["target/**".to_string()],
        plugins: Vec::new(),
    };
    let config = evaluator_thread_config(&agent, &full_scope(), None, &agent.thinking);
    let root_permissions = config["permissions"]["canon_check"]["filesystem"][":project_roots"]
        .as_object()
        .unwrap();
    assert_eq!(root_permissions["."], "read");
    assert_eq!(root_permissions[".canon"], "none");
    assert_eq!(root_permissions[".canon/**"], "none");
    assert_eq!(root_permissions[".git/canon"], "none");
    assert_eq!(root_permissions[".git/canon/**"], "none");
    assert_eq!(root_permissions[".git/canon/logs"], "none");
    assert_eq!(root_permissions[".git/canon/logs/**"], "none");
    assert_eq!(root_permissions["target"], "none");
    assert_eq!(root_permissions["target/**"], "none");
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"][":root"],
        "read"
    );
    assert_eq!(config["model_reasoning_effort"], "low");
    assert!(config["permissions"]["canon_check"]["filesystem"]
        .get("~/.codex/tmp/**")
        .is_none());
    assert!(config["permissions"]["canon_check"]["filesystem"]
        .get(":tmpdir")
        .is_none());
    assert!(config["permissions"]["canon_check"]["filesystem"]
        .get(":slash_tmp")
        .is_none());
    assert!(config["permissions"]["canon_check"]["filesystem"]
        .get("/private/tmp/**")
        .is_none());
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["~/.codex/sessions"],
        "none"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["~/.codex/sessions/**"],
        "none"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["~/.codex/memories"],
        "none"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["~/.codex/memories/**"],
        "none"
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
fn restricted_evaluator_scope_is_enforced_by_filesystem_permissions() {
    let agent = AgentConfig {
        model: ModelConfig::default(),
        thinking: "low".to_string(),
        instructions: Some("Answer from files only.".to_string()),
        ignore: vec!["target/**".to_string()],
        plugins: Vec::new(),
    };
    let root_permissions = evaluator_thread_root_permissions(&agent, &["src".to_string()]);

    assert_eq!(root_permissions["."], "none");
    assert_eq!(root_permissions["src"], "read");
    assert_eq!(root_permissions["src/**"], "read");
    assert_eq!(root_permissions[".canon"], "none");
    assert_eq!(root_permissions[".canon/**"], "none");
    assert_eq!(root_permissions["target"], "none");
    assert_eq!(root_permissions["target/**"], "none");

    let file_scope_permissions =
        evaluator_thread_root_permissions(&agent, &["src/bin/main.rs".to_string()]);
    assert_eq!(file_scope_permissions["."], "none");
    assert_eq!(file_scope_permissions["src"], "read");
    assert_eq!(file_scope_permissions["src/bin"], "read");
    assert_eq!(file_scope_permissions["src/bin/main.rs"], "read");
    assert_eq!(file_scope_permissions["src/bin/main.rs/**"], "read");
    assert!(!file_scope_permissions.contains_key("src/**"));
    assert!(!file_scope_permissions.contains_key("src/bin/**"));
}

#[test]
fn evaluator_model_is_configured_when_present() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let thread_config =
        evaluator_thread_config(&config.agent, &full_scope(), None, &config.agent.thinking);
    assert_eq!(thread_config["model"], "gpt-5.4-mini");
    let fallback_config = evaluator_thread_config(
        &config.agent,
        &full_scope(),
        Some("gpt-5.3-codex-spark"),
        &config.agent.thinking,
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
    let thread_config =
        evaluator_thread_config(&config.agent, &full_scope(), None, &config.agent.thinking);
    assert_eq!(
        thread_config["plugins"]["canon@codex-plugins"]["enabled"],
        json!(true)
    );
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
    let canonical_root = root.canonicalize().unwrap();
    let model_catalog_path = resolve_git_path(&canonical_root, "canon/evaluator-model-catalogs")
        .unwrap()
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
    assert!(filesystem_arg.contains(r#"":project_roots"={"."="none""#));
    assert!(filesystem_arg.contains(r#"".canon/**"="none""#));
    assert!(filesystem_arg.contains(r#""target"="none""#));
    assert!(filesystem_arg.contains(r#""target/**"="none""#));
    assert!(filesystem_arg.contains(r#"":root"="read""#));
    assert!(!filesystem_arg.contains(r#"":tmpdir""#));
    assert!(!filesystem_arg.contains(r#"":slash_tmp""#));
    assert!(!filesystem_arg.contains(r#""/private/tmp/**""#));
    assert!(!filesystem_arg.contains(r#""~/.codex/tmp/**""#));
    assert!(filesystem_arg.contains(r#""glob_scan_max_depth"=32"#));
    assert!(filesystem_arg.contains(r#""~/.codex/sessions"="none""#));
    assert!(filesystem_arg.contains(r#""~/.codex/sessions/**"="none""#));
    assert!(filesystem_arg.contains(r#""~/.codex/memories"="none""#));
    assert!(filesystem_arg.contains(r#""~/.codex/memories/**"="none""#));
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

    assert_eq!(
        evaluator_home,
        env::temp_dir()
            .canonicalize()
            .unwrap()
            .join("canon")
            .join(".codex")
    );
    assert!(!evaluator_home.starts_with(&root));
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
fn evaluator_codex_home_rejects_symlinked_temp_parent() {
    use std::os::unix::fs::symlink;

    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CODEX_HOME", "TMPDIR"]);
    env_snapshot.remove("CODEX_HOME");
    let temp_root = TestDir::new("codex-home-temp-root");
    let target_root = TestDir::new("codex-home-temp-target");
    symlink(target_root.path(), temp_root.path().join("canon")).unwrap();
    env_snapshot.set("TMPDIR", temp_root.path());
    let root = git_project("app-server-symlinked-temp-canon");

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
    env_snapshot.set("CANON_TEST_SECRET", "secret-token");
    env_snapshot.set("CODEX_HOME", "/tmp/source-codex-home");
    env_snapshot.set("CODEX_THREAD_ID", "parent-thread");
    env_snapshot.set("HOME", "/tmp/real-home");
    env_snapshot.set("PATH", "/bin:/usr/bin");
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
        Some("/bin:/usr/bin")
    );
    assert!(envs.contains_key("TMPDIR"));
}

#[test]
fn app_server_startup_config_escapes_toml_control_characters() {
    let agent = AgentConfig {
        model: ModelConfig::default(),
        thinking: "low".to_string(),
        instructions: Some("Answer from files only.".to_string()),
        ignore: vec![
            "quoted\"path/**".to_string(),
            "control\u{0007}path/**".to_string(),
            "delete\u{007f}path/**".to_string(),
        ],
        plugins: Vec::new(),
    };

    let filesystem_arg = app_server_startup_filesystem_arg(&agent);

    assert!(filesystem_arg.contains(r#""quoted\"path"="none""#));
    assert!(filesystem_arg.contains(r#""control\u0007path"="none""#));
    assert!(filesystem_arg.contains(r#""delete\u007Fpath"="none""#));
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
