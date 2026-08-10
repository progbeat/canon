use super::environment::{
    configured_dir, configured_secret_dir, make_temp_dir, SandboxDir, CANON_SANDBOX_DIR,
};
use super::secret_dir::{chmod_secret_dir_no_access, SecretDirConfig, SecretDirModeRestoration};
use crate::platform::filesystem;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) struct NaiveIsolationPolicy {
    secret_dir: SecretDirConfig,
    pub(super) sandbox_dir: Arc<SandboxDir>,
    pub(super) counter: u64,
}

impl NaiveIsolationPolicy {
    pub(crate) fn from_env() -> Result<NaiveIsolationPolicy, String> {
        let secret_dir = SecretDirConfig::from_path(configured_secret_dir())?;
        let (sandbox_dir, remove_sandbox_dir_on_drop) = match configured_dir(CANON_SANDBOX_DIR) {
            Some(path) => (path, false),
            None => (make_temp_dir()?, true),
        };
        Ok(NaiveIsolationPolicy {
            secret_dir,
            sandbox_dir: Arc::new(SandboxDir {
                path: sandbox_dir,
                remove_on_drop: remove_sandbox_dir_on_drop,
            }),
            counter: 0,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_dirs(
        secret_dir: Option<PathBuf>,
        sandbox_dir: PathBuf,
    ) -> Result<NaiveIsolationPolicy, String> {
        filesystem::create_private_dir_all(&sandbox_dir).map_err(|err| {
            format!(
                "failed to create test sandbox dir {}: {}",
                sandbox_dir.display(),
                err
            )
        })?;
        Ok(NaiveIsolationPolicy {
            secret_dir: SecretDirConfig::from_path(secret_dir)?,
            sandbox_dir: Arc::new(SandboxDir {
                path: sandbox_dir,
                remove_on_drop: false,
            }),
            counter: 0,
        })
    }

    pub(crate) fn isolate(&mut self, path: &Path) -> Result<NaiveIsolationGuard, String> {
        let original_path = path.to_path_buf();
        if let Some(secret_dir) = self.secret_dir.boundary() {
            // xpec: Hj
            assert!(
                is_subpath(&original_path, secret_dir)?,
                "cannot isolate path {} outside of secret dir {}",
                original_path.display(),
                secret_dir.display()
            );
        }
        let isolated_path = self.next_isolated_path();
        filesystem::move_path(&original_path, &isolated_path)?;
        let mut guard = NaiveIsolationGuard {
            original_path,
            isolated_path,
            _sandbox_dir: Arc::clone(&self.sandbox_dir),
            secret_dir_mode_restoration: None,
            active: true,
        };
        if let Some(restoration) = self.secret_dir.permission_restoration() {
            if let Err(err) = chmod_secret_dir_no_access(&restoration.path) {
                let restore_err = guard.restore().err();
                return Err(match restore_err {
                    Some(restore_err) => {
                        format!("{}; also failed to restore isolation: {}", err, restore_err)
                    }
                    None => err,
                });
            }
            guard.secret_dir_mode_restoration = Some(restoration);
        }
        Ok(guard)
    }

    pub(super) fn next_isolated_path(&mut self) -> PathBuf {
        let isolated_path = self.sandbox_dir.path.join(format!("{:X}", self.counter));
        // Match the policy order: derive the destination from the current
        // counter, increment it, then assert that the destination does not
        // already exist. Rust's `Path::exists`, like the policy's
        // `os.path.exists`, follows symlinks: a dangling destination symlink is
        // therefore absent to this assertion, leaving the subsequent move to
        // apply its ordinary platform behavior.
        self.counter += 1;
        // xpec: Hj
        assert!(
            !isolated_path.exists(),
            "counter collision in sandbox isolation: {}",
            isolated_path.display()
        );
        isolated_path
    }
}

pub(crate) struct NaiveIsolationGuard {
    original_path: PathBuf,
    isolated_path: PathBuf,
    _sandbox_dir: Arc<SandboxDir>,
    secret_dir_mode_restoration: Option<SecretDirModeRestoration>,
    active: bool,
}

impl NaiveIsolationGuard {
    pub(crate) fn path(&self) -> &Path {
        &self.isolated_path
    }

    fn restore(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let mut errors = Vec::new();
        if let Some(restoration) = &self.secret_dir_mode_restoration {
            if let Err(err) =
                filesystem::restore_secret_dir_mode(&restoration.path, &restoration.mode)
            {
                errors.push(format!(
                    "failed to restore secret dir permissions {}: {}",
                    restoration.path.display(),
                    err
                ));
            }
        }
        match filesystem::move_path(&self.isolated_path, &self.original_path) {
            Ok(()) => {
                self.active = false;
            }
            Err(err) => {
                errors.push(err);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

impl Drop for NaiveIsolationGuard {
    fn drop(&mut self) {
        if let Err(err) = self.restore() {
            panic!("failed to restore naive isolation: {err}");
        }
    }
}

fn is_subpath(path: &Path, dir: &Path) -> Result<bool, String> {
    Ok(path.starts_with(dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: Hj
    fn naive_isolation_asserts_destination_collision() {
        let root = test_root("naive-isolation-destination-collision");
        let sandbox = root.join("sandbox");
        filesystem::create_private_dir_all(&sandbox.join("0")).unwrap();
        let mut policy = NaiveIsolationPolicy::with_dirs(None, sandbox).unwrap();

        let collision = catch_unwind(AssertUnwindSafe(|| policy.next_isolated_path()));

        assert!(collision.is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test] // xpec: Hj
    fn naive_isolation_collision_check_treats_dangling_symlink_as_absent() {
        let root = test_root("naive-isolation-dangling-destination");
        let sandbox = root.join("sandbox");
        let project = root.join("repository");
        filesystem::create_private_dir_all(&project).unwrap();
        let mut policy = NaiveIsolationPolicy::with_dirs(None, sandbox.clone()).unwrap();
        symlink(root.join("missing"), sandbox.join("0")).unwrap();

        let err = match policy.isolate(&project) {
            Ok(_) => panic!("moving a directory over a dangling symlink should fail"),
            Err(err) => err,
        };

        assert!(!err.contains("counter collision in sandbox isolation"));
        assert!(err.contains("failed to move isolated path"));
        assert!(project.is_dir());
        assert!(fs::symlink_metadata(sandbox.join("0"))
            .unwrap()
            .file_type()
            .is_symlink());
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: Hj
    fn temporary_sandbox_outlives_policy_while_guard_is_active() {
        let root = test_root("naive-isolation-sandbox-owner");
        let sandbox = root.join("sandbox");
        let project = root.join("repository");
        filesystem::create_private_dir_all(&project).unwrap();
        let mut policy = NaiveIsolationPolicy::with_dirs(None, sandbox.clone()).unwrap();
        policy.sandbox_dir = Arc::new(SandboxDir {
            path: sandbox.clone(),
            remove_on_drop: true,
        });

        let guard = policy.isolate(&project).unwrap();
        drop(policy);

        assert!(guard.path().is_dir());
        assert!(sandbox.is_dir());
        drop(guard);
        assert!(project.is_dir());
        assert!(!sandbox.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: Hj
    fn restoration_failure_propagates_from_guard_drop() {
        let root = test_root("naive-isolation-restore-failure");
        let sandbox = root.join("sandbox");
        let project = root.join("repository");
        filesystem::create_private_dir_all(&project).unwrap();
        let mut policy = NaiveIsolationPolicy::with_dirs(None, sandbox).unwrap();
        let guard = policy.isolate(&project).unwrap();
        let isolated_path = guard.path().to_path_buf();
        filesystem::create_private_dir_all(&project).unwrap();
        fs::write(project.join("replacement"), "occupied").unwrap();

        let restore_failure = catch_unwind(AssertUnwindSafe(|| drop(guard)));

        assert!(restore_failure.is_err());
        assert!(isolated_path.is_dir());
        let _ = filesystem::make_directory_tree_private(&isolated_path);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test] // xpec: Hj
    fn naive_isolation_moves_path_to_sandbox_and_restores_on_drop() {
        let root = test_root("naive-isolation-restore");
        let secret = root.join("secret");
        let sandbox = root.join("sandbox");
        let project = secret.join("repository");
        filesystem::create_private_dir_all(&project).unwrap();
        fs::write(project.join("file.txt"), "visible").unwrap();
        let mut policy =
            NaiveIsolationPolicy::with_dirs(Some(secret.clone()), sandbox.clone()).unwrap();

        {
            let guard = policy.isolate(&project).unwrap();
            assert_eq!(guard.path(), sandbox.join("0"));
            assert!(!project.exists());
            assert!(guard.path().join("file.txt").is_file());
            assert_eq!(dir_mode(&secret), 0o000);
        }

        assert!(project.join("file.txt").is_file());
        assert_eq!(dir_mode(&secret), 0o700);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test] // xpec: Hj
    fn naive_isolation_secret_dir_operations_follow_symlink() {
        let root = test_root("naive-isolation-secret-symlink");
        let secret_target = root.join("secret-target");
        let secret_link = root.join("secret-link");
        let sandbox = root.join("sandbox");
        let project = secret_link.join("repository");
        filesystem::create_private_dir_all(&secret_target.join("repository")).unwrap();
        symlink(&secret_target, &secret_link).unwrap();
        let mut policy =
            NaiveIsolationPolicy::with_dirs(Some(secret_link), sandbox.clone()).unwrap();

        {
            let guard = policy.isolate(&project).unwrap();
            assert_eq!(guard.path(), sandbox.join("0"));
            assert_eq!(dir_mode(&secret_target), 0o000);
        }

        assert!(project.is_dir());
        assert_eq!(dir_mode(&secret_target), 0o700);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test] // xpec: Hj
    fn secret_dir_restoration_resolves_retargeted_symlink() {
        let root = test_root("naive-isolation-secret-retarget");
        let first_target = root.join("first-target");
        let second_target = root.join("second-target");
        let secret_link = root.join("secret-link");
        let sandbox = root.join("sandbox");
        let project = secret_link.join("repository");
        filesystem::create_private_dir_all(&first_target.join("repository")).unwrap();
        filesystem::create_private_dir_all(&second_target).unwrap();
        fs::set_permissions(&second_target, fs::Permissions::from_mode(0o500)).unwrap();
        symlink(&first_target, &secret_link).unwrap();
        let mut policy =
            NaiveIsolationPolicy::with_dirs(Some(secret_link.clone()), sandbox).unwrap();
        let guard = policy.isolate(&project).unwrap();

        fs::remove_file(&secret_link).unwrap();
        symlink(&second_target, &secret_link).unwrap();
        drop(guard);

        assert_eq!(dir_mode(&first_target), 0o000);
        assert_eq!(dir_mode(&second_target), 0o700);
        assert!(second_target.join("repository").is_dir());
        fs::set_permissions(&first_target, fs::Permissions::from_mode(0o700)).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test] // xpec: Hj
    fn naive_isolation_preserves_read_only_root_permissions() {
        let root = test_root("naive-isolation-read-only-root");
        let sandbox = root.join("sandbox");
        let project = root.join("repository");
        filesystem::create_private_dir_all(&project).unwrap();
        fs::write(project.join("file.txt"), "visible").unwrap();
        filesystem::set_materialized_dir_permissions(&project).unwrap();
        let mut policy = NaiveIsolationPolicy::with_dirs(None, sandbox.clone()).unwrap();

        {
            let guard = policy.isolate(&project).unwrap();
            assert_eq!(guard.path(), sandbox.join("0"));
            assert_eq!(dir_mode(guard.path()), 0o555);
        }

        assert!(project.join("file.txt").is_file());
        assert_eq!(dir_mode(&project), 0o555);
        let _ = filesystem::make_directory_tree_private(&project);
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: Hj
    fn naive_isolation_asserts_paths_are_inside_secret_dir() {
        let root = test_root("naive-isolation-secret-boundary");
        let secret = root.join("secret");
        let sandbox = root.join("sandbox");
        let outside = root.join("outside");
        filesystem::create_private_dir_all(&secret).unwrap();
        filesystem::create_private_dir_all(&outside).unwrap();
        let mut policy = NaiveIsolationPolicy::with_dirs(Some(secret), sandbox).unwrap();

        let boundary_violation = catch_unwind(AssertUnwindSafe(|| policy.isolate(&outside)));

        assert!(boundary_violation.is_err());
        assert!(outside.exists());
        let _ = fs::remove_dir_all(root);
    }

    fn test_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("canon-test-{}-{}-{}", name, process::id(), unique));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[cfg(unix)]
    fn dir_mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
    }
}
