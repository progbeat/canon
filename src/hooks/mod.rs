mod fs;
mod git;

// This module manages Git pre-commit hook installation only.
use self::fs::make_executable;
use self::fs::HookFile;
use self::git::{git_hooks_path_matches, HookPreflight};
use crate::fs_util::{ensure_dir_without_symlinks, remove_optional_file, replace_file};
use crate::output::{write_stderr, write_stdout, write_stdout_line};
use clap::error::ErrorKind as ClapErrorKind;
use clap::Command as ClapCommand;
use std::ffi::OsString;
use std::path::Path;

// Canon installs the hook at Git's default repository hook path and reports
// that same path through the public command output.
pub(super) const DEFAULT_GIT_HOOKS_PATH: &str = ".git/hooks";
pub(super) const DEFAULT_GIT_PRE_COMMIT_HOOK_PATH: &str = ".git/hooks/pre-commit";
pub(super) const DEFAULT_GIT_PRE_COMMIT_HOOK_COMMON_DIR_PATH: &str = "hooks/pre-commit";
const DEFAULT_PRE_COMMIT_HOOK: &str = include_str!("pre_commit_hook.sh");
const PRE_COMMIT_HOOK_MANUAL_ADVICE: &str =
    "Can't safely install pre-commit hook.\n▷ Add `canon gate` manually to the existing pre-commit setup or ask a human to handle it.";
const PRE_COMMIT_HOOK_UNINSTALL_MANUAL_ADVICE: &str =
    "Can't safely uninstall pre-commit hook because it is not Canon's hook.\n▷ Remove it manually only if intended, or ask a human to handle it.";
pub(super) const GIT_WORKTREE_REQUIRED_FOR_HOOK_INSTALL: &str =
    "Can't safely install pre-commit hook: canon pre-commit install requires a Git worktree.";

pub(crate) fn pre_commit_help_command() -> ClapCommand {
    ClapCommand::new("pre-commit")
        .bin_name("canon pre-commit")
        .about("Manage the canon pre-commit hook")
        .subcommand_required(true)
        .subcommand(ClapCommand::new("install").about("Install the canon pre-commit hook"))
        .subcommand(ClapCommand::new("uninstall").about("Uninstall the canon pre-commit hook"))
}

pub(crate) fn run_pre_commit_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    let Some(action) = parse_pre_commit_action(args)? else {
        return Ok(());
    };
    match action {
        HookAction::Install => run_hook_install(root),
        HookAction::Uninstall => run_hook_uninstall(root),
    }
}

enum HookAction {
    Install,
    Uninstall,
}

fn parse_pre_commit_action(args: &[OsString]) -> Result<Option<HookAction>, String> {
    let argv = std::iter::once(OsString::from("canon pre-commit"))
        .chain(args.iter().cloned())
        .collect::<Vec<_>>();
    let matches = match pre_commit_help_command().try_get_matches_from(argv) {
        Ok(matches) => matches,
        Err(err)
            if matches!(
                err.kind(),
                ClapErrorKind::DisplayHelp | ClapErrorKind::DisplayVersion
            ) =>
        {
            write_clap_display_error(&err)?;
            return Ok(None);
        }
        Err(err) => return Err(err.to_string()),
    };
    match matches.subcommand_name() {
        Some("install") => Ok(Some(HookAction::Install)),
        Some("uninstall") => Ok(Some(HookAction::Uninstall)),
        Some(action) => Err(format!("unknown pre-commit command: {}", action)),
        None => unreachable!("clap requires a pre-commit subcommand"),
    }
}

fn write_clap_display_error(err: &clap::Error) -> Result<(), String> {
    let rendered = err.to_string();
    if err.use_stderr() {
        write_stderr(&rendered)
    } else {
        write_stdout(&rendered)
    }
}

pub(crate) fn run_hook_install(root: &Path) -> Result<(), String> {
    let preflight = HookPreflight::load(root)?;
    preflight_git_worktree(&preflight)?;
    preflight_pre_commit_hook_ownership(&preflight, HookAction::Install)?;
    preflight_git_hooks_path(&preflight)?;
    install_pre_commit_hook(&preflight)
}

fn preflight_git_worktree(preflight: &HookPreflight) -> Result<(), String> {
    if preflight.is_git_worktree {
        return Ok(());
    }
    Err(GIT_WORKTREE_REQUIRED_FOR_HOOK_INSTALL.to_string())
}

fn preflight_pre_commit_hook_ownership(
    preflight: &HookPreflight,
    action: HookAction,
) -> Result<(), String> {
    // The default Git hook path is user-owned whenever Git is not already
    // Canon's exact hook. A matching `core.hooksPath` identifies the directory,
    // not ownership of a file already present in that directory.
    match &preflight.pre_commit_hook {
        HookFile::Missing => Ok(()),
        HookFile::Regular(contents) if contents == DEFAULT_PRE_COMMIT_HOOK => Ok(()),
        HookFile::Regular(_) | HookFile::Unverifiable => match action {
            HookAction::Install => Err(pre_commit_hook_manual_advice()),
            HookAction::Uninstall => Err(PRE_COMMIT_HOOK_UNINSTALL_MANUAL_ADVICE.to_string()),
        },
    }
}

fn preflight_git_hooks_path(preflight: &HookPreflight) -> Result<(), String> {
    // [D8] Canon owns the hook directory only when effective Git configuration
    // has no custom hook manager or already points at Canon's hook directory.
    // `git config` precedence includes system, global, worktree, and local
    // sources; any effective non-Canon value needs manual handling.
    if let Some(existing) = &preflight.effective_git_hooks_path {
        if !git_hooks_path_matches(&preflight.root, &preflight.git_hooks_path, existing) {
            return Err(pre_commit_hook_manual_advice());
        }
    }
    Ok(())
}

fn install_pre_commit_hook(preflight: &HookPreflight) -> Result<(), String> {
    let hook_path = preflight.pre_commit_hook_path.as_path();
    if let Some(parent) = hook_path.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    replace_file(hook_path, DEFAULT_PRE_COMMIT_HOOK)?;
    make_executable(hook_path)?;
    write_stdout_line(&format!("Installed {}", DEFAULT_GIT_PRE_COMMIT_HOOK_PATH))
}

pub(crate) fn run_hook_uninstall(root: &Path) -> Result<(), String> {
    let preflight = HookPreflight::load(root)?;
    // [D8] Verify file ownership before removing Canon's hook file. A foreign
    // hook must remain untouched.
    preflight_pre_commit_hook_ownership(&preflight, HookAction::Uninstall)?;
    remove_optional_file(&preflight.pre_commit_hook_path)?;
    // The hook file has been removed by this point, and there is no later
    // uninstall work to delay the flushed success line.
    write_stdout_line(&format!("Uninstalled {}", DEFAULT_GIT_PRE_COMMIT_HOOK_PATH))
}

fn pre_commit_hook_manual_advice() -> String {
    PRE_COMMIT_HOOK_MANUAL_ADVICE.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn preflight(
        pre_commit_hook: HookFile,
        effective_git_hooks_path: Option<&str>,
    ) -> HookPreflight {
        let root = PathBuf::from("repo");
        HookPreflight {
            effective_git_hooks_path: effective_git_hooks_path.map(str::to_string),
            pre_commit_hook,
            is_git_worktree: true,
            git_hooks_path: root.join(DEFAULT_GIT_HOOKS_PATH),
            pre_commit_hook_path: root.join(DEFAULT_GIT_PRE_COMMIT_HOOK_PATH),
            root,
        }
    }

    #[test] // xpec: D8
    fn foreign_pre_commit_hook_requires_documented_manual_setup() {
        let preflight = preflight(HookFile::Unverifiable, None);

        let error =
            preflight_pre_commit_hook_ownership(&preflight, HookAction::Install).unwrap_err();

        assert_eq!(error, PRE_COMMIT_HOOK_MANUAL_ADVICE);
    }

    #[test] // xpec: D8
    fn external_hook_manager_requires_documented_manual_setup() {
        let preflight = preflight(HookFile::Missing, Some("global-hooks"));

        let error = preflight_git_hooks_path(&preflight).unwrap_err();

        assert_eq!(error, PRE_COMMIT_HOOK_MANUAL_ADVICE);
    }

    #[test] // xpec: D8
    fn install_requires_a_git_worktree() {
        let mut preflight = preflight(HookFile::Missing, None);
        preflight.is_git_worktree = false;

        let error = preflight_git_worktree(&preflight).unwrap_err();

        assert_eq!(error, GIT_WORKTREE_REQUIRED_FOR_HOOK_INSTALL);
    }

    #[test] // xpec: KD,D8
    fn install_and_uninstall_preserve_user_owned_local_hooks_path() {
        let root = test_repo("preserve-user-hooks-path");
        git(
            &root,
            &["config", "--local", "core.hooksPath", ".git/hooks"],
        );

        run_hook_install(&root).unwrap();
        assert_eq!(
            local_git_config_value(&root, "core.hooksPath").as_deref(),
            Some(".git/hooks")
        );
        run_hook_uninstall(&root).unwrap();

        assert_eq!(
            local_git_config_value(&root, "core.hooksPath").as_deref(),
            Some(".git/hooks")
        );
        assert!(!root.join(DEFAULT_GIT_PRE_COMMIT_HOOK_PATH).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: KD,D8
    fn install_uses_the_default_hook_path_without_creating_git_config() {
        let root = test_repo("default-hook-path");

        run_hook_install(&root).unwrap();

        assert_eq!(local_git_config_value(&root, "core.hooksPath"), None);
        assert_eq!(
            fs::read_to_string(root.join(DEFAULT_GIT_PRE_COMMIT_HOOK_PATH)).unwrap(),
            DEFAULT_PRE_COMMIT_HOOK
        );
        run_hook_uninstall(&root).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    fn test_repo(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("canon-hooks-{name}-{}-{unique}", process::id()));
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "--quiet"]);
        root
    }

    fn git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .unwrap();
        // xpec: KD,D8
        assert!(status.success(), "git {args:?} failed");
    }

    fn local_git_config_value(root: &Path, key: &str) -> Option<String> {
        let output = Command::new("git")
            .args(["config", "--local", "--null", "--get", key])
            .current_dir(root)
            .output()
            .unwrap();
        if output.status.code() == Some(1) && output.stderr.is_empty() {
            return None;
        }
        if !output.status.success() {
            panic!(
                "git config failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let value = output
            .stdout
            .strip_suffix(b"\0")
            .expect("git config value must be NUL-terminated");
        Some(
            std::str::from_utf8(value)
                .expect("git config value must be UTF-8")
                .to_string(),
        )
    }
}
