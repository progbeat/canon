use std::env;
use std::path::Path;
use std::process::Command;

pub(crate) fn configure_app_server_environment(
    command: &mut Command,
    isolated_codex_home: &Path,
    isolated_temp_root: &Path,
) -> Result<(), String> {
    let path = env::var_os("PATH");
    command.env_clear();
    if let Some(path) = path {
        command.env("PATH", path);
    }
    // This is the cwd of the app-server transport/control process, not an
    // evaluator agent. Each evaluator agent is started later by thread/start,
    // whose cwd is supplied independently from the checked runtime.
    command.current_dir(isolated_temp_root);
    command.env("CODEX_HOME", isolated_codex_home);
    if let Some(home) = isolated_codex_home.parent() {
        command.env("HOME", home);
    }
    for key in ["TMPDIR", "TEMP", "TMP"] {
        command.env(key, isolated_temp_root);
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};

    #[test] // xpec: A8,bP
    fn app_server_environment_uses_isolated_codex_home() {
        let mut command = Command::new("codex");
        let codex_home = Path::new("/tmp/canon-evaluator-home/.codex");
        let temp_root = codex_home.parent().unwrap();

        configure_app_server_environment(&mut command, codex_home, temp_root).unwrap();

        assert_eq!(command.get_current_dir(), Some(temp_root));
        assert_eq!(
            command_env_value(&command, "CODEX_HOME"),
            Some(codex_home.as_os_str().to_os_string())
        );
        assert_eq!(
            command_env_value(&command, "HOME"),
            Some(codex_home.parent().unwrap().as_os_str().to_os_string())
        );
        for key in ["TMPDIR", "TEMP", "TMP"] {
            assert_eq!(
                command_env_value(&command, key),
                Some(temp_root.as_os_str().to_os_string())
            );
        }
    }

    fn command_env_value(command: &Command, key: &str) -> Option<OsString> {
        command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new(key))
            .and_then(|(_, value)| value.map(OsStr::to_os_string))
    }
}
