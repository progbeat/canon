use super::*;

pub(crate) fn git_project(name: &str) -> PathBuf {
    let root = temp_home(name);
    Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap();
    for args in [
        ["config", "core.autocrlf", "false"],
        ["config", "core.eol", "lf"],
    ] {
        let output = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git config failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::write(root.join("README.md"), "hello").unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&root)
        .output()
        .unwrap();
    root
}

pub(crate) fn commit_all(root: &Path, message: &str) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=Canon Test",
            "-c",
            "user.email=canon@example.test",
            "commit",
            "-m",
            message,
        ])
        .current_dir(root)
        .output()
        .expect("run git commit");
    assert!(
        output.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(crate) fn write_check_config(root: &Path) {
    fs::create_dir_all(root.join(".canon")).unwrap();
    fs::write(root.join(CHECK_PATH), check_config_yaml()).unwrap();
}
