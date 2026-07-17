use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn configure_app_server_environment(
    command: &mut Command,
    isolated_codex_home: &Path,
) -> Result<(), String> {
    let path = env::var_os("PATH");
    let temp_root = app_server_process_temp_root()?;
    command.env_clear();
    if let Some(path) = path {
        command.env("PATH", path);
    }
    // This is the cwd of the app-server transport/control process, not an
    // evaluator agent. Each evaluator agent is started later by thread/start,
    // whose cwd is supplied independently from the checked runtime.
    command.current_dir(&temp_root);
    command.env("CODEX_HOME", isolated_codex_home);
    if let Some(home) = isolated_codex_home.parent() {
        command.env("HOME", home);
    }
    for key in ["TMPDIR", "TEMP", "TMP"] {
        command.env(key, &temp_root);
    }
    Ok(())
}

fn app_server_process_temp_root() -> Result<PathBuf, String> {
    env::temp_dir()
        .canonicalize()
        .map_err(|err| format!("failed to canonicalize temp dir: {}", err))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};

    #[test]
    fn app_server_environment_uses_isolated_codex_home() {
        let mut command = Command::new("codex");
        let codex_home = Path::new("/tmp/canon-evaluator-home/.codex");

        configure_app_server_environment(&mut command, codex_home).unwrap();

        assert_eq!(
            command.get_current_dir(),
            Some(env::temp_dir().canonicalize().unwrap().as_path())
        );
        assert_eq!(
            command_env_value(&command, "CODEX_HOME"),
            Some(codex_home.as_os_str().to_os_string())
        );
        assert_eq!(
            command_env_value(&command, "HOME"),
            Some(codex_home.parent().unwrap().as_os_str().to_os_string())
        );
    }

    fn command_env_value(command: &Command, key: &str) -> Option<OsString> {
        command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new(key))
            .and_then(|(_, value)| value.map(OsStr::to_os_string))
    }
}
