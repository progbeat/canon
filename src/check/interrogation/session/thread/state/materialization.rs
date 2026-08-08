//! Materialized workspace ownership for evaluator threads.

use super::ThreadState;
use crate::isolation::NaiveIsolationGuard;
use std::path::{Path, PathBuf};

pub(super) enum MaterializedSessionRoot {
    Direct,
    Isolated(NaiveIsolationGuard),
}

impl MaterializedSessionRoot {
    fn path<'a>(&'a self, canonical_root: &'a Path) -> &'a Path {
        match self {
            Self::Direct => canonical_root,
            Self::Isolated(restoration) => restoration.path(),
        }
    }
}

impl ThreadState {
    pub(in crate::check::interrogation::session::thread) fn prepare_materialized_session_root(
        &mut self,
        canonical_root: &Path,
        materialize: impl FnOnce() -> Result<PathBuf, String>,
    ) -> Result<PathBuf, String> {
        if let Some(root) = self.materialized_session_roots.get(canonical_root) {
            return Ok(root.path(canonical_root).to_path_buf());
        }
        let materialized_root = materialize()?;
        assert_eq!(
            materialized_root, canonical_root,
            "materialized evaluator root must use its canonical tree path"
        ); // xpec: Hj
        let root = match self.isolation_policy.as_mut() {
            Some(policy) => MaterializedSessionRoot::Isolated(policy.isolate(&materialized_root)?),
            None => MaterializedSessionRoot::Direct,
        };
        let session_root = root.path(canonical_root).to_path_buf();
        self.materialized_session_roots
            .insert(materialized_root, root);
        Ok(session_root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolation::NaiveIsolationPolicy;
    use std::cell::Cell;
    use std::fs;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: d
    fn direct_materialized_root_is_cached_across_thread_reset() {
        let canonical_root = PathBuf::from("/canonical/tree");
        let mut thread_state = ThreadState::new(
            true,
            crate::platform::filesystem::PrivateTemporaryDirectoryAllocator::new(),
        )
        .unwrap();
        let materializations = Cell::new(0);

        let _first = thread_state
            .prepare_materialized_session_root(&canonical_root, || {
                materializations.set(materializations.get() + 1);
                Ok(canonical_root.clone())
            })
            .unwrap();
        thread_state.clear_threads();
        let _second = thread_state
            .prepare_materialized_session_root(&canonical_root, || {
                panic!("a direct root must not be materialized again")
            })
            .unwrap();

        assert_eq!(materializations.get(), 1);
    }

    #[test] // xpec: Hj,A8,KD
    fn materialized_sessions_move_caller_cache_outside_protected_source_root() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "canon-shared-isolated-root-{}-{unique}",
            process::id()
        ));
        let protected_home = root.join("home");
        let canonical_root = protected_home.join("repo/.cache/trees/tree-oid");
        let sandbox = root.join("sandbox");
        crate::platform::filesystem::create_private_dir_all(&canonical_root).unwrap();
        let mut thread_state = ThreadState::new(
            true,
            crate::platform::filesystem::PrivateTemporaryDirectoryAllocator::new(),
        )
        .unwrap();
        thread_state.isolation_policy = Some(
            NaiveIsolationPolicy::with_dirs(None, sandbox.clone()).expect("test isolation policy"),
        );
        let materializations = Cell::new(0);

        let first = thread_state
            .prepare_materialized_session_root(&canonical_root, || {
                materializations.set(materializations.get() + 1);
                Ok(canonical_root.clone())
            })
            .unwrap();
        thread_state.clear_threads();
        let second = thread_state
            .prepare_materialized_session_root(&canonical_root, || {
                panic!("an isolated canonical root must not be materialized again")
            })
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(first, sandbox.join("0"));
        assert_eq!(materializations.get(), 1);
        assert!(!canonical_root.exists());
        assert!(!first.starts_with(&protected_home));
        assert!(first.is_dir());
        drop(thread_state);
        assert!(canonical_root.is_dir());
        assert!(sandbox.is_dir());
        let _ = fs::remove_dir_all(root);
    }
}
