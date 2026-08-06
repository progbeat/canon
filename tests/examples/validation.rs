use super::*;

// xpec: 1h,w
#[test]
fn invalid_check_arguments_still_emit_the_check_trailer() {
    let repo = portable_temp_dir("canon-invalid-check-argument-trailer");
    fs::create_dir_all(&repo).unwrap();
    let output = canon()
        .args(["check", "--unknown-check-option"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(" 0 passed in "));
    assert!(!stdout.contains("✓ All checks passed."));
    assert!(!stdout.contains("▷ Run `canon check`"));
    let stderr = String::from_utf8(output.stderr).unwrap();
    // xpec: 1h,w
    let diagnostic_offset = stderr.find("unexpected argument").unwrap();
    let trailer_offset = stderr.find(ZERO_TOKEN_USAGE_LINE).unwrap();
    assert!(diagnostic_offset < trailer_offset, "{stderr}");
}

// xpec: 1h,w
#[test]
fn state_root_resolution_failure_still_emits_the_check_trailer() {
    let repo = committed_git_project("canon-state-root-failure-trailer").unwrap();
    let output = canon()
        .arg("check")
        .current_dir(&repo)
        .env("CANON_STATE_DIR", "")
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(" 0 passed in "), "{stdout}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let diagnostic_offset = stderr.find("CANON_STATE_DIR must not be empty").unwrap();
    let trailer_offset = stderr.find(ZERO_TOKEN_USAGE_LINE).unwrap();
    assert!(diagnostic_offset < trailer_offset, "{stderr}");
}

// xpec: 2Z,w
#[test]
fn missing_check_config_requires_a_fix_instead_of_promising_continuation() {
    let repo = committed_git_project("canon-missing-config-feedback").unwrap();
    let output = canon().arg("check").current_dir(&repo).output().unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("No canon check config found at .canon/check.yml"),
        "{stdout}\n{stderr}"
    );
    assert!(stderr.contains("Run `canon init`"), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("▷ Fix the reported error and run `canon check` again!"),
        "{stdout}\n{stderr}"
    );
    assert!(
        !stdout.contains("continue evaluation"),
        "{stdout}\n{stderr}"
    );
}

// xpec: l
#[test]
fn invalid_ask_arguments_still_emit_token_usage() {
    let repo = portable_temp_dir("canon-invalid-ask-argument-token-usage");
    fs::create_dir_all(&repo).unwrap();
    let output = canon()
        .args(["ask", "Can this pass?", "--keep-going"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.starts_with(ZERO_TOKEN_USAGE_LINE), "{stderr}");
    assert!(stderr.contains("unexpected argument"), "{stderr}");
}

// xpec: 2Z,l,Pi
#[test]
fn git_backed_ask_setup_failure_reports_its_cause() {
    let repo = committed_git_project("canon-ask-setup-failure-diagnostic").unwrap();
    let missing_secret_dir = repo.join("missing-secret");
    let output = canon()
        .args(["ask", "Does setup failure stay visible?"])
        .current_dir(&repo)
        .env("CANON_SECRET_DIR", &missing_secret_dir)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.starts_with(ZERO_TOKEN_USAGE_LINE), "{stderr}");
    assert!(
        stderr.contains("Error: failed to inspect secret directory"),
        "{stderr}"
    );
    assert!(stderr.contains("missing-secret"), "{stderr}");
    assert!(!stderr.contains("canon ask failed"), "{stderr}");
}

// xpec: 2Z,l,Pi
#[test]
fn ask_evaluator_failure_is_reported_once_in_query_output() {
    let repo = portable_temp_dir("canon-ask-evaluator-failure-output");
    fs::create_dir_all(&repo).unwrap();
    let output = canon()
        .args(["ask", "--in-place", "Does evaluator failure stay visible?"])
        .current_dir(&repo)
        .env("PATH", path_with_failing_codex(&repo))
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.starts_with("q."), "{stdout}\n{stderr}");
    assert!(stdout.contains("error: unparsable"), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("evidence:") && stdout.contains("unknown command: debug"),
        "{stdout}\n{stderr}"
    );
    assert_eq!(stderr, ZERO_TOKEN_USAGE_LINE);
    assert!(!stderr.contains("canon ask failed"), "{stdout}\n{stderr}");
}

// xpec: 2Z,l,Pi
#[test]
fn ask_output_failure_is_reported_on_stderr() {
    let repo = portable_temp_dir("canon-ask-output-failure-diagnostic");
    fs::create_dir_all(&repo).unwrap();
    let mut child = canon()
        .args(["ask", "--in-place", "Does output failure stay visible?"])
        .current_dir(&repo)
        .env("PATH", path_with_failing_codex(&repo))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.starts_with(ZERO_TOKEN_USAGE_LINE), "{stderr}");
    assert!(
        stderr.contains("Error: failed to flush") && stderr.contains("to stdout"),
        "{stderr}"
    );
    assert!(!stderr.contains("canon ask failed"), "{stderr}");
}

fn path_with_failing_codex(repo: &std::path::Path) -> std::ffi::OsString {
    let tools = repo.join("tools");
    fs::create_dir_all(&tools).unwrap();
    let codex_name = format!("codex{}", std::env::consts::EXE_SUFFIX);
    fs::copy(env!("CARGO_BIN_EXE_canon"), tools.join(codex_name)).unwrap();
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(std::iter::once(tools).chain(std::env::split_paths(&original_path)))
        .unwrap()
}

// xpec: 1h,w,KD
#[test]
fn collected_check_failure_reports_pending_count() {
    let repo = portable_temp_dir("canon-collected-check-failure");
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        "presets:\n  default: {}\nexpectations:\n  - to: caller\n    q: \"Answer yes:\"\n    a: \"yes\"\n",
    )
    .unwrap();
    let output = canon()
        .args(["check", "--in-place", "unknown-selector"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains(" 1 pending in "), "{stdout}\n{stderr}");
    // xpec: 1h,w
    let diagnostic_offset = stderr.find("expectation").unwrap();
    let trailer_offset = stderr.find(ZERO_TOKEN_USAGE_LINE).unwrap();
    assert!(diagnostic_offset < trailer_offset, "{stderr}");
}

// xpec: 2Z,w
#[test]
fn identity_validation_failure_reports_pending_and_requires_a_fix() {
    let repo = portable_temp_dir("canon-collected-identity-failure");
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
presets:
  default: {}
expectations:
  - q: "Duplicate expectation"
    a: "yes"
  - q: "Duplicate expectation"
    a: "yes"
"#,
    )
    .unwrap();
    for args in [&["init", "--quiet"][..], &["add", "."][..]] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
    }
    let output = canon().arg("check").current_dir(&repo).output().unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains(" 2 pending in "), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("▷ Fix the reported error and run `canon check` again!"),
        "{stdout}\n{stderr}"
    );
    assert!(
        !stdout.contains("continue evaluation"),
        "{stdout}\n{stderr}"
    );
    assert!(stderr.contains("duplicate expectation ID"), "{stderr}");
}
