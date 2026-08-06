use super::*;

#[test] // xpec: KD,cw,90
fn in_place_results_cannot_hide_a_git_backed_regression_from_gate() {
    let repo = portable_temp_dir("canon-in-place-preserves-gate-history");
    fs::create_dir_all(repo.join(".canon")).unwrap();
    let git_config = r#"
presets:
  default: {}
expectations:
  - to: caller
    q: "Type first:"
    a: "pass"
    rank: 0
  - to: caller
    q: "Type second:"
    a: "pass"
    rank: 1
"#;
    fs::write(repo.join(".canon/check.yml"), git_config).unwrap();
    fs::write(repo.join("tracked.txt"), "baseline\n").unwrap();
    for args in [
        &["init", "--quiet"][..],
        &["add", "."][..],
        &[
            "-c",
            "user.name=Canon Test",
            "-c",
            "user.email=canon@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "baseline",
        ][..],
    ] {
        Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .unwrap();
    }

    let pass = run_caller_check_without_state_override(&repo, &["check"], b"pass\npass\n");
    assert!(pass.status.success(), "{pass:?}");
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
presets:
  default: {}
expectations:
  - to: caller
    q: "Type first:"
    a: "pass"
    rank: 0
"#,
    )
    .unwrap();
    let in_place_pass =
        run_caller_check_without_state_override(&repo, &["check", "--in-place"], b"pass\n");
    assert!(in_place_pass.status.success(), "{in_place_pass:?}");
    fs::write(repo.join(".canon/check.yml"), git_config).unwrap();
    fs::write(repo.join("tracked.txt"), "regression\n").unwrap();
    Command::new("git")
        .args(["add", "tracked.txt"])
        .current_dir(&repo)
        .status()
        .unwrap();
    let staged_fail = run_caller_check_without_state_override(&repo, &["check"], b"pass\nfail\n");
    assert!(!staged_fail.status.success(), "{staged_fail:?}");

    let gate = canon()
        .arg("gate")
        .current_dir(&repo)
        .env_remove("CANON_STATE_DIR")
        .output()
        .unwrap();

    assert!(!gate.status.success(), "{gate:?}");
    let gate_stderr = String::from_utf8(gate.stderr).unwrap();
    assert!(
        gate_stderr.contains("staged changes regress cached canon results"),
        "{gate_stderr}"
    );
    let _ = fs::remove_dir_all(&repo);
}

#[test] // xpec: KD,cw,fh
fn gate_preserves_pass_state_for_an_xpec_added_by_the_staged_config() {
    let repo = portable_temp_dir("canon-gate-preserves-staged-config-state");
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
presets:
  default: {}
expectations:
  - to: caller
    q: "Type first:"
    a: "pass"
    rank: 0
"#,
    )
    .unwrap();
    fs::write(repo.join("tracked.txt"), "baseline\n").unwrap();
    for args in [
        &["init", "--quiet"][..],
        &["add", "."][..],
        &[
            "-c",
            "user.name=Canon Test",
            "-c",
            "user.email=canon@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "baseline",
        ][..],
    ] {
        let status = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .unwrap();
        assert!(status.success());
    }
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
presets:
  default: {}
expectations:
  - to: caller
    q: "Type first:"
    a: "pass"
    rank: 0
  - to: caller
    q: "Type second:"
    a: "pass"
    rank: 1
"#,
    )
    .unwrap();
    let status = Command::new("git")
        .args(["add", ".canon/check.yml"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(status.success());
    let staged_config_pass =
        run_caller_check_without_state_override(&repo, &["check"], b"pass\npass\n");
    assert!(
        staged_config_pass.status.success(),
        "{staged_config_pass:?}"
    );

    let gate_before_commit = canon()
        .arg("gate")
        .current_dir(&repo)
        .env_remove("CANON_STATE_DIR")
        .output()
        .unwrap();
    assert!(
        gate_before_commit.status.success(),
        "{gate_before_commit:?}"
    );
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=Canon Test",
            "-c",
            "user.email=canon@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "add second xpec",
        ])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(status.success());

    fs::write(repo.join("tracked.txt"), "regression\n").unwrap();
    let status = Command::new("git")
        .args(["add", "tracked.txt"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(status.success());
    let staged_fail = run_caller_check_without_state_override(&repo, &["check"], b"pass\nfail\n");
    assert!(!staged_fail.status.success(), "{staged_fail:?}");
    let gate_after_regression = canon()
        .arg("gate")
        .current_dir(&repo)
        .env_remove("CANON_STATE_DIR")
        .output()
        .unwrap();

    assert!(
        !gate_after_regression.status.success(),
        "{gate_after_regression:?}"
    );
    let gate_stderr = String::from_utf8(gate_after_regression.stderr).unwrap();
    assert!(
        gate_stderr.contains("staged changes regress cached canon results"),
        "{gate_stderr}"
    );
    let _ = fs::remove_dir_all(&repo);
}

fn run_caller_check_without_state_override(
    repo: &std::path::Path,
    args: &[&str],
    answer: &[u8],
) -> std::process::Output {
    let mut child = canon()
        .args(args)
        .current_dir(repo)
        .env_remove("CANON_STATE_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(answer).unwrap();
    child.wait_with_output().unwrap()
}
