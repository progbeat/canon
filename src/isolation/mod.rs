use crate::platform;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

const CANON_SECRET_DIR: &str = "CANON_SECRET_DIR";
const CANON_SANDBOX_DIR: &str = "CANON_SANDBOX_DIR";

pub(crate) struct NaiveIsolationPolicy {
    secret_dir: Option<PathBuf>,
    sandbox_dir: PathBuf,
    remove_sandbox_dir_on_drop: bool,
    counter: u64,
    secret_dir_mode: Option<platform::SecretDirMode>,
}

impl NaiveIsolationPolicy {
    pub(crate) fn from_env() -> Result<NaiveIsolationPolicy, String> {
        let secret_dir = configured_secret_dir();
        let (sandbox_dir, remove_sandbox_dir_on_drop) = match configured_dir(CANON_SANDBOX_DIR) {
            Some(path) => {
                platform::create_private_dir_all(&path).map_err(|err| {
                    format!(
                        "failed to create {} {}: {}",
                        CANON_SANDBOX_DIR,
                        path.display(),
                        err
                    )
                })?;
                (path, false)
            }
            None => (make_temp_dir()?, true),
        };
        let secret_dir_mode = secret_dir.as_deref().map(stat_mode).transpose()?;
        Ok(NaiveIsolationPolicy {
            secret_dir,
            sandbox_dir,
            remove_sandbox_dir_on_drop,
            counter: 0,
            secret_dir_mode,
        })
    }

    #[cfg(test)]
    fn with_dirs(
        secret_dir: Option<PathBuf>,
        sandbox_dir: PathBuf,
    ) -> Result<NaiveIsolationPolicy, String> {
        platform::create_private_dir_all(&sandbox_dir).map_err(|err| {
            format!(
                "failed to create test sandbox dir {}: {}",
                sandbox_dir.display(),
                err
            )
        })?;
        let secret_dir_mode = secret_dir.as_deref().map(stat_mode).transpose()?;
        Ok(NaiveIsolationPolicy {
            secret_dir,
            sandbox_dir,
            remove_sandbox_dir_on_drop: false,
            counter: 0,
            secret_dir_mode,
        })
    }

    pub(crate) fn isolate(&mut self, path: &Path) -> Result<NaiveIsolationGuard, String> {
        let original_path = path.to_path_buf();
        if let Some(secret_dir) = &self.secret_dir {
            if !is_subpath(&original_path, secret_dir)? {
                return Err(format!(
                    "cannot isolate path {} outside of secret dir {}",
                    original_path.display(),
                    secret_dir.display()
                ));
            }
        }
        let isolated_path = self.next_isolated_path()?;
        platform::move_path(&original_path, &isolated_path)?;
        let guard = NaiveIsolationGuard {
            original_path,
            isolated_path,
            secret_dir: self.secret_dir.clone(),
            secret_dir_mode: self.secret_dir_mode.clone(),
            hidden_root_mode: None,
            active: true,
        };
        if let Some(secret_dir) = &guard.secret_dir {
            if let Err(err) = chmod_secret_dir_no_access(secret_dir) {
                let mut guard = guard;
                let restore_err = guard.restore().err();
                return Err(match restore_err {
                    Some(restore_err) => {
                        format!("{}; also failed to restore isolation: {}", err, restore_err)
                    }
                    None => err,
                });
            }
        }
        Ok(guard)
    }

    fn next_isolated_path(&mut self) -> Result<PathBuf, String> {
        let isolated_path = self.sandbox_dir.join(format!("{:X}", self.counter));
        // Match the policy order: derive the destination from the current
        // counter, increment it, then assert that the destination does not
        // already exist.
        self.counter += 1;
        if isolated_path.exists() {
            Err(format!(
                "counter collision in sandbox isolation: {}",
                isolated_path.display()
            ))
        } else {
            Ok(isolated_path)
        }
    }
}

impl Drop for NaiveIsolationPolicy {
    fn drop(&mut self) {
        if self.remove_sandbox_dir_on_drop {
            let _ = fs::remove_dir_all(&self.sandbox_dir);
        }
    }
}

pub(crate) struct NaiveIsolationGuard {
    original_path: PathBuf,
    isolated_path: PathBuf,
    secret_dir: Option<PathBuf>,
    secret_dir_mode: Option<platform::SecretDirMode>,
    hidden_root_mode: Option<platform::SecretDirMode>,
    active: bool,
}

impl NaiveIsolationGuard {
    pub(crate) fn path(&self) -> &Path {
        &self.isolated_path
    }

    pub(crate) fn hide(&mut self) -> Result<(), String> {
        if !self.active || self.hidden_root_mode.is_some() {
            return Ok(());
        }
        let mode = dir_mode(&self.isolated_path)?;
        chmod_dir_no_access(&self.isolated_path)?;
        self.hidden_root_mode = Some(mode);
        Ok(())
    }

    pub(crate) fn reveal(&mut self) -> Result<(), String> {
        let Some(mode) = self.hidden_root_mode.take() else {
            return Ok(());
        };
        match restore_dir_mode(&self.isolated_path, &mode) {
            Ok(()) => Ok(()),
            Err(err) => {
                self.hidden_root_mode = Some(mode);
                Err(err)
            }
        }
    }

    fn restore(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let mut errors = Vec::new();
        if let Err(err) = self.reveal() {
            errors.push(format!(
                "failed to reveal isolated path {} before restore: {}",
                self.isolated_path.display(),
                err
            ));
        }
        if let (Some(secret_dir), Some(mode)) = (&self.secret_dir, self.secret_dir_mode.clone()) {
            if let Err(err) = platform::restore_secret_dir_mode(secret_dir, &mode) {
                errors.push(format!(
                    "failed to restore secret dir permissions {}: {}",
                    secret_dir.display(),
                    err
                ));
            }
        }
        match platform::move_path(&self.isolated_path, &self.original_path) {
            Ok(()) => {
                self.active = false;
            }
            Err(err) => errors.push(err),
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
        let _ = self.restore();
    }
}

fn configured_secret_dir() -> Option<PathBuf> {
    let value = env::var_os(CANON_SECRET_DIR)?;
    // Match the policy's `if self.secret_dir` branches: an empty environment
    // value is falsey and therefore behaves like no configured secret dir.
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

fn configured_dir(name: &str) -> Option<PathBuf> {
    let value = env::var_os(name)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

fn is_subpath(path: &Path, dir: &Path) -> Result<bool, String> {
    Ok(path.starts_with(dir))
}

fn make_temp_dir() -> Result<PathBuf, String> {
    let parent = env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    for attempt in 0..1000 {
        let path = parent.join(format!(
            "canon-sandbox-{}-{}-{}",
            process::id(),
            stamp,
            attempt
        ));
        match platform::create_private_dir(&path) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(format!("failed to create {}: {}", path.display(), err)),
        }
    }
    Err(format!(
        "failed to allocate a unique sandbox directory under {}",
        parent.display()
    ))
}

fn stat_mode(path: &Path) -> Result<platform::SecretDirMode, String> {
    platform::secret_dir_mode(path)
}

fn chmod_secret_dir_no_access(path: &Path) -> Result<(), String> {
    platform::chmod_secret_dir_no_access(path)
}

fn dir_mode(path: &Path) -> Result<platform::SecretDirMode, String> {
    platform::secret_dir_mode(path)
}

fn chmod_dir_no_access(path: &Path) -> Result<(), String> {
    platform::chmod_secret_dir_no_access(path)
}

fn restore_dir_mode(path: &Path, mode: &platform::SecretDirMode) -> Result<(), String> {
    platform::restore_secret_dir_mode(path, mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    #[test]
    fn naive_isolation_moves_path_to_sandbox_and_restores_on_drop() {
        let root = test_root("naive-isolation-restore");
        let secret = root.join("secret");
        let sandbox = root.join("sandbox");
        let project = secret.join("repository");
        platform::create_private_dir_all(&project).unwrap();
        fs::write(project.join("file.txt"), "visible").unwrap();
        let mut policy =
            NaiveIsolationPolicy::with_dirs(Some(secret.clone()), sandbox.clone()).unwrap();

        {
            let guard = policy.isolate(&project).unwrap();
            assert_eq!(guard.path(), sandbox.join("0"));
            assert!(!project.exists());
            assert!(guard.path().join("file.txt").is_file());
            #[cfg(unix)]
            assert_eq!(dir_mode(&secret), 0o000);
        }

        assert!(project.join("file.txt").is_file());
        #[cfg(unix)]
        assert_eq!(dir_mode(&secret), 0o700);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn naive_isolation_preserves_read_only_root_permissions() {
        let root = test_root("naive-isolation-read-only-root");
        let sandbox = root.join("sandbox");
        let project = root.join("repository");
        platform::create_private_dir_all(&project).unwrap();
        fs::write(project.join("file.txt"), "visible").unwrap();
        platform::set_materialized_dir_permissions(&project).unwrap();
        let mut policy = NaiveIsolationPolicy::with_dirs(None, sandbox.clone()).unwrap();

        {
            let guard = policy.isolate(&project).unwrap();
            assert_eq!(guard.path(), sandbox.join("0"));
            #[cfg(unix)]
            assert_eq!(dir_mode(guard.path()), 0o555);
        }

        assert!(project.join("file.txt").is_file());
        #[cfg(unix)]
        assert_eq!(dir_mode(&project), 0o555);
        let _ = platform::set_private_dir_permissions(&project);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn naive_isolation_can_hide_and_reveal_parked_roots() {
        let root = test_root("naive-isolation-hide-parked-root");
        let sandbox = root.join("sandbox");
        let first_project = root.join("first");
        let second_project = root.join("second");
        platform::create_private_dir_all(&first_project).unwrap();
        platform::create_private_dir_all(&second_project).unwrap();
        fs::write(first_project.join("file.txt"), "first").unwrap();
        fs::write(second_project.join("file.txt"), "second").unwrap();
        platform::set_materialized_dir_permissions(&first_project).unwrap();
        platform::set_materialized_dir_permissions(&second_project).unwrap();
        let mut policy = NaiveIsolationPolicy::with_dirs(None, sandbox).unwrap();

        {
            let mut first = policy.isolate(&first_project).unwrap();
            let second = policy.isolate(&second_project).unwrap();

            first.hide().unwrap();
            assert_eq!(dir_mode(first.path()), 0o000);
            assert_eq!(dir_mode(second.path()), 0o555);

            first.reveal().unwrap();
            assert_eq!(dir_mode(first.path()), 0o555);
            assert_eq!(
                fs::read_to_string(first.path().join("file.txt")).unwrap(),
                "first"
            );
        }

        assert_eq!(dir_mode(&first_project), 0o555);
        assert_eq!(dir_mode(&second_project), 0o555);
        let _ = platform::set_private_dir_permissions(&first_project);
        let _ = platform::set_private_dir_permissions(&second_project);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn naive_isolation_rejects_paths_outside_secret_dir() {
        let root = test_root("naive-isolation-secret-boundary");
        let secret = root.join("secret");
        let sandbox = root.join("sandbox");
        let outside = root.join("outside");
        platform::create_private_dir_all(&secret).unwrap();
        platform::create_private_dir_all(&outside).unwrap();
        let mut policy = NaiveIsolationPolicy::with_dirs(Some(secret), sandbox).unwrap();

        let err = match policy.isolate(&outside) {
            Ok(_) => panic!("outside path should be rejected"),
            Err(err) => err,
        };

        assert!(err.contains("outside of secret dir"), "{err}");
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
