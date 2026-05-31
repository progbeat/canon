use crate::platform;
use std::env;
use std::fs;
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

const CANON_SECRET_DIR: &str = "CANON_SECRET_DIR";
const CANON_SANDBOX_DIR: &str = "CANON_SANDBOX_DIR";

pub(crate) struct NaiveIsolationPolicy {
    secret_dir: Option<PathBuf>,
    sandbox_dir: PathBuf,
    counter: u64,
    secret_dir_mode: Option<fs::Permissions>,
}

impl NaiveIsolationPolicy {
    pub(crate) fn from_env() -> Result<NaiveIsolationPolicy, String> {
        let secret_dir = configured_secret_dir()
            .map(|path| canonical_dir(&path, CANON_SECRET_DIR))
            .transpose()?;
        let sandbox_dir = match configured_dir(CANON_SANDBOX_DIR) {
            Some(path) => {
                platform::create_private_dir_all(&path).map_err(|err| {
                    format!(
                        "failed to create {} {}: {}",
                        CANON_SANDBOX_DIR,
                        path.display(),
                        err
                    )
                })?;
                canonical_dir(&path, CANON_SANDBOX_DIR)?
            }
            None => make_temp_dir()?,
        };
        let secret_dir_mode = secret_dir.as_deref().map(stat_mode).transpose()?;
        Ok(NaiveIsolationPolicy {
            secret_dir,
            sandbox_dir,
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
        let secret_dir = secret_dir
            .map(|path| canonical_dir(&path, "test secret dir"))
            .transpose()?;
        let secret_dir_mode = secret_dir.as_deref().map(stat_mode).transpose()?;
        Ok(NaiveIsolationPolicy {
            secret_dir,
            sandbox_dir: canonical_dir(&sandbox_dir, "test sandbox dir")?,
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

pub(crate) struct NaiveIsolationGuard {
    original_path: PathBuf,
    isolated_path: PathBuf,
    secret_dir: Option<PathBuf>,
    secret_dir_mode: Option<fs::Permissions>,
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
        if let (Some(secret_dir), Some(mode)) = (&self.secret_dir, self.secret_dir_mode.clone()) {
            if let Err(err) = fs::set_permissions(secret_dir, mode) {
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
    let path = path.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize isolation path {}: {}",
            path.display(),
            err
        )
    })?;
    let dir = dir.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize secret dir {}: {}",
            dir.display(),
            err
        )
    })?;
    Ok(path.starts_with(dir))
}

fn canonical_dir(path: &Path, description: &str) -> Result<PathBuf, String> {
    let canonical = path.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize {} {}: {}",
            description,
            path.display(),
            err
        )
    })?;
    if !canonical.is_dir() {
        return Err(format!(
            "{} {} is not a directory",
            description,
            canonical.display()
        ));
    }
    Ok(canonical)
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

fn stat_mode(path: &Path) -> Result<fs::Permissions, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| format!("failed to stat secret dir {}: {}", path.display(), err))?;
    if !metadata.file_type().is_dir() {
        return Err(format!("secret dir {} is not a directory", path.display()));
    }
    Ok(metadata.permissions())
}

fn chmod_secret_dir_no_access(path: &Path) -> Result<(), String> {
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("naive isolation requires Unix chmod support for CANON_SECRET_DIR".to_string())
    }
    #[cfg(unix)]
    {
        let mut permissions = stat_mode(path)?;
        permissions.set_mode(0o000);
        fs::set_permissions(path, permissions)
            .map_err(|err| format!("failed to chmod secret dir {}: {}", path.display(), err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
