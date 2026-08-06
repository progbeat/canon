use super::*;

// xpec: 1h,2Z,KD,w
#[test]
fn state_write_failure_after_pass_reports_the_error_without_success_feedback() {
    let repo = portable_temp_dir("canon-pass-state-write-failure");
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
presets:
  default: {}
expectations:
  - to: caller
    q: "Answer pass:"
    a: "pass"
"#,
    )
    .unwrap();
    fs::write(repo.join("tracked.txt"), "baseline\n").unwrap();
    for args in [
        &["init", "--quiet"][..],
        &["add", ".canon/check.yml", "tracked.txt"][..],
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

    let state_root = repo.join("configured-state");
    let mut child = canon()
        .arg("check")
        .current_dir(&repo)
        .env("CANON_STATE_DIR", &state_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout_reader = child.stdout.take().unwrap();
    let mut prompt = vec![0; "Answer pass: ".len()];
    stdout_reader.read_exact(&mut prompt).unwrap();
    assert_eq!(prompt, b"Answer pass: ");

    if state_root.is_dir() {
        fs::remove_dir_all(&state_root).unwrap();
    } else if state_root.exists() {
        fs::remove_file(&state_root).unwrap();
    }
    fs::write(&state_root, "blocks result persistence\n").unwrap();
    child.stdin.take().unwrap().write_all(b"pass\n").unwrap();

    let mut stdout_tail = Vec::new();
    stdout_reader.read_to_end(&mut stdout_tail).unwrap();
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    let status = child.wait().unwrap();
    let stdout = String::from_utf8([prompt, stdout_tail].concat()).unwrap();
    let stderr = String::from_utf8(stderr).unwrap();

    assert!(!status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains(". OK\n"), "{stdout}\n{stderr}");
    assert!(stdout.contains(" 1 passed in "), "{stdout}\n{stderr}");
    assert!(!stdout.contains("All checks passed"), "{stdout}\n{stderr}");
    assert!(
        !stdout.contains("Commit the staged changes"),
        "{stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("Fix the reported error and run `canon check` again"),
        "{stdout}\n{stderr}"
    );
    assert!(stderr.contains("Error:"), "{stdout}\n{stderr}");
    let _ = fs::remove_dir_all(repo);
}
