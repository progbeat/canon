use super::{path_to_config_string, EvaluatorConfigError, EvaluatorConfigResult};
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::path::Path;

pub(super) const FILESYSTEM_DENY: &str = "deny";
pub(super) const EVALUATOR_FILESYSTEM_GLOB_SCAN_MAX_DEPTH: u64 = 32;

pub(crate) fn evaluator_working_tree_permissions(
    session_root: &Path,
) -> EvaluatorConfigResult<BTreeMap<String, String>> {
    let mut permissions = BTreeMap::new();
    insert_filesystem_permission(
        &mut permissions,
        absolute_session_path(session_root, ".")?,
        "read",
    )?;
    insert_filesystem_permission(
        &mut permissions,
        absolute_session_glob(session_root, "**")?,
        "read",
    )?;
    Ok(permissions)
}

pub(crate) fn evaluator_template_output_permissions(
    template_output_dir: &Path,
) -> EvaluatorConfigResult<BTreeMap<String, String>> {
    let mut permissions = BTreeMap::new();
    insert_filesystem_permission(
        &mut permissions,
        path_to_config_string(template_output_dir, "evaluator template output dir")?,
        "read",
    )?;
    insert_filesystem_permission(
        &mut permissions,
        path_to_config_string(
            &template_output_dir.join("**"),
            "evaluator template output dir glob",
        )?,
        "read",
    )?;
    Ok(permissions)
}

pub(crate) fn evaluator_resolved_state_dir_permissions(
    state_root: &Path,
) -> EvaluatorConfigResult<BTreeMap<String, String>> {
    let mut permissions = BTreeMap::new();
    insert_tree_permission(&mut permissions, &state_root, FILESYSTEM_DENY)?;
    Ok(permissions)
}

fn insert_tree_permission(
    permissions: &mut BTreeMap<String, String>,
    path: &Path,
    permission: &str,
) -> EvaluatorConfigResult<()> {
    let path = path_to_config_string(path, "evaluator filesystem permission path")?;
    let path = path.trim_end_matches('/').to_string();
    insert_filesystem_permission(permissions, path.clone(), permission)?;
    insert_filesystem_permission(permissions, format!("{}/**", path), permission)?;
    Ok(())
}

pub(super) fn merge_filesystem_permissions(
    target: &mut BTreeMap<String, String>,
    source: BTreeMap<String, String>,
) -> EvaluatorConfigResult<()> {
    for (path, permission) in source {
        insert_filesystem_permission(target, path, &permission)?;
    }
    Ok(())
}

fn insert_filesystem_permission(
    permissions: &mut BTreeMap<String, String>,
    path: String,
    permission: &str,
) -> EvaluatorConfigResult<()> {
    if let Some(existing) = permissions.get(&path) {
        return Err(EvaluatorConfigError::DuplicateFilesystemPermission {
            path,
            existing: existing.clone(),
            replacement: permission.to_string(),
        });
    }
    permissions.insert(path, permission.to_string());
    Ok(())
}

fn absolute_session_path(session_root: &Path, path: &str) -> EvaluatorConfigResult<String> {
    let path = if path == "." {
        session_root.to_path_buf()
    } else {
        session_root.join(path)
    };
    path_to_config_string(&path, "evaluator session path")
}

fn absolute_session_glob(session_root: &Path, pattern: &str) -> EvaluatorConfigResult<String> {
    path_to_config_string(&session_root.join(pattern), "evaluator session glob path")
}

pub(crate) fn evaluator_runtime_permissions() -> EvaluatorConfigResult<BTreeMap<String, String>> {
    let mut permissions = BTreeMap::new();
    for path in [
        "~",
        "~/.zlogin",
        "~/.zlogout",
        "~/.zprofile",
        "~/.zshenv",
        "~/.zshrc",
        "/etc/**",
        "/private/etc/**",
        "/bin/**",
        "/usr/bin/**",
        "/usr/lib/**",
        "/usr/libexec/**",
        "/usr/share/**",
        "/System/**",
        "/Library/**",
        "/opt/homebrew/**",
    ] {
        insert_filesystem_permission(&mut permissions, path.to_string(), "read")?;
    }
    deny_runtime_path(&mut permissions, ":tmpdir")?;
    deny_runtime_path(&mut permissions, ":slash_tmp")?;
    deny_runtime_path(&mut permissions, "/dev/null")?;
    deny_runtime_tree(&mut permissions, "/tmp")?;
    deny_runtime_tree(&mut permissions, "/private/tmp")?;
    deny_runtime_tree(&mut permissions, "~/.codex/sessions")?;
    deny_runtime_tree(&mut permissions, "~/.codex/memories")?;
    add_home_runtime_permissions(&mut permissions, env::var_os("HOME"))?;
    Ok(permissions)
}

fn add_home_runtime_permissions(
    permissions: &mut BTreeMap<String, String>,
    home: Option<OsString>,
) -> EvaluatorConfigResult<()> {
    if let Some(home) = home {
        let home = home
            .into_string()
            .map_err(|_| EvaluatorConfigError::HomeNotUtf8)?;
        let codex_home = format!("{}/.codex", home.trim_end_matches('/'));
        deny_runtime_tree(permissions, &format!("{}/sessions", codex_home))?;
        deny_runtime_tree(permissions, &format!("{}/memories", codex_home))?;
    }
    Ok(())
}

fn deny_runtime_path(
    permissions: &mut BTreeMap<String, String>,
    path: &str,
) -> EvaluatorConfigResult<()> {
    insert_filesystem_permission(permissions, path.to_string(), FILESYSTEM_DENY)
}

fn deny_runtime_tree(
    permissions: &mut BTreeMap<String, String>,
    path: &str,
) -> EvaluatorConfigResult<()> {
    deny_runtime_path(permissions, path)?;
    deny_runtime_path(permissions, &format!("{}/**", path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_permissions_deny_common_temp_entry_points() {
        let permissions = evaluator_runtime_permissions().unwrap();

        for path in [
            ":tmpdir",
            ":slash_tmp",
            "/dev/null",
            "/tmp",
            "/tmp/**",
            "/private/tmp",
            "/private/tmp/**",
        ] {
            assert_permission(&permissions, path, FILESYSTEM_DENY);
        }
    }

    #[cfg(unix)]
    #[test]
    fn runtime_permissions_reject_non_utf8_home() {
        use std::os::unix::ffi::OsStringExt;

        let mut permissions = BTreeMap::new();
        let home = OsString::from_vec(b"/tmp/canon-home-\xff".to_vec());

        assert!(add_home_runtime_permissions(&mut permissions, Some(home)).is_err());
        assert!(permissions.is_empty());
    }

    #[test]
    fn working_tree_permissions_read_session_root_and_children() {
        let session_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("canon-materialized-tree");
        let permissions = evaluator_working_tree_permissions(&session_root).unwrap();
        let root_key = path_to_config_string(&session_root, "test session root").unwrap();
        let children_key =
            path_to_config_string(&session_root.join("**"), "test session children").unwrap();

        assert_eq!(permissions.get(&root_key), Some(&"read".to_string()));
        assert_eq!(permissions.get(&children_key), Some(&"read".to_string()));
    }

    #[test]
    fn template_output_permissions_read_output_dir_and_children() {
        let output_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("canon-template-output");
        let permissions = evaluator_template_output_permissions(&output_dir).unwrap();
        let root_key = path_to_config_string(&output_dir, "test output dir").unwrap();
        let children_key =
            path_to_config_string(&output_dir.join("**"), "test output children").unwrap();

        assert_eq!(permissions.get(&root_key), Some(&"read".to_string()));
        assert_eq!(permissions.get(&children_key), Some(&"read".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn working_tree_permissions_reject_non_utf8_session_root() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let session_root = Path::new(OsStr::from_bytes(b"/tmp/canon-\xff"));

        assert!(evaluator_working_tree_permissions(session_root).is_err());
    }

    #[test]
    fn state_dir_permissions_deny_canon_state_tree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let state_root =
            crate::git::resolve_git_path(root, crate::state_paths::CANON_STATE_DIR_GIT_PATH)
                .unwrap();
        let state_root_key = path_to_config_string(&state_root, "test state root").unwrap();
        let permissions = evaluator_resolved_state_dir_permissions(&state_root).unwrap();

        assert_eq!(
            permissions.get(&state_root_key),
            Some(&FILESYSTEM_DENY.to_string())
        );
        assert_eq!(
            permissions.get(&format!("{}/**", state_root_key)),
            Some(&FILESYSTEM_DENY.to_string())
        );
    }

    fn assert_permission(permissions: &BTreeMap<String, String>, path: &str, expected: &str) {
        assert_eq!(permissions.get(path), Some(&expected.to_string()));
    }
}
