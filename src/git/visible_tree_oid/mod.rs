use super::program::TrackedFile;
use super::tree_source::TreeSource;
use crate::config_types::AgentConfig;
use crate::repo_inspection::RepoInspectionCache;
use crate::scope::{effective_ignore_patterns, path_bytes_in_scope, visible_scope};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

mod cache;
mod hash;
mod scope_entries;

pub(crate) use hash::git_object_oid_has_known_shape;
use hash::{git_object_hash_algorithm, GitObjectHashAlgorithm};
use scope_entries::{
    visible_scope_entries_from_files, visible_tree_oid_from_files,
    visible_tree_oid_from_files_if_scope_present,
};

// This module implements only the `visibleTreeOid` fingerprint. Persistent
// per-expectation result state lives under `xpec_state`.
// [d] The derived-hash key contains the repository root, resolved tree-source
// identity, requested scope, and effective ignore patterns: every input that
// selects entries for a visible-tree OID within the invocation snapshot. The
// source-file snapshot and repository hash algorithm are memoized once by
// their own root/source keys.
type SourceScopeCacheKey = (PathBuf, String, Vec<String>, Vec<String>);
type SourceCacheKey = (PathBuf, String);
type StoredVisibleTreeOidCache = Rc<RefCell<BTreeMap<Vec<String>, Result<String, String>>>>;

fn cached_value<K: Ord, V: Clone>(
    cache: &RefCell<BTreeMap<K, V>>,
    key: K,
    compute: impl FnOnce() -> Result<V, String>,
) -> Result<V, String> {
    if let Some(cached) = cache.borrow().get(&key).cloned() {
        return Ok(cached);
    }
    let computed = compute()?;
    cache.borrow_mut().insert(key, computed.clone());
    Ok(computed)
}

pub(crate) struct VisibleTreeOidCache {
    required_visible_tree_oids: Rc<RefCell<BTreeMap<SourceScopeCacheKey, String>>>,
    visible_tree_oids: Rc<RefCell<BTreeMap<SourceScopeCacheKey, Option<String>>>>,
    visible_scope_entries: Rc<RefCell<BTreeMap<SourceScopeCacheKey, Vec<String>>>>,
    stored_visible_scope_resolvers:
        Rc<RefCell<BTreeMap<SourceCacheKey, StoredVisibleScopeOidResolver>>>,
    repo_inspection: RepoInspectionCache,
    object_hash_algorithms: Rc<RefCell<BTreeMap<PathBuf, GitObjectHashAlgorithm>>>,
}

#[derive(Clone)]
pub(crate) struct StoredVisibleScopeOidResolver {
    files: Rc<Vec<TrackedFile>>,
    object_hash_algorithm: GitObjectHashAlgorithm,
    visible_tree_oids: StoredVisibleTreeOidCache,
}

impl StoredVisibleScopeOidResolver {
    pub(crate) fn oid_for_stored_visible_scope(
        &self,
        stored_visible_scope: &[String],
    ) -> Result<String, String> {
        if let Some(cached) = self
            .visible_tree_oids
            .borrow()
            .get(stored_visible_scope)
            .cloned()
        {
            return cached;
        }
        let result = visible_tree_oid_from_files(
            &self.files,
            stored_visible_scope,
            self.object_hash_algorithm,
        );
        self.visible_tree_oids
            .borrow_mut()
            .insert(stored_visible_scope.to_vec(), result.clone());
        result
    }
}

fn source_scope_cache_key_parts(
    root: &Path,
    agent: &AgentConfig,
    scope: &[String],
) -> Result<(PathBuf, Vec<String>, Vec<String>), String> {
    let mut ignore_patterns = effective_ignore_patterns(agent)?;
    ignore_patterns.sort();
    ignore_patterns.dedup();
    Ok((root.to_path_buf(), scope.to_vec(), ignore_patterns))
}

fn source_scope_cache_key(
    root: &Path,
    source: &TreeSource,
    agent: &AgentConfig,
    scope: &[String],
) -> Result<SourceScopeCacheKey, String> {
    let (root, scope, ignore_patterns) = source_scope_cache_key_parts(root, agent, scope)?;
    Ok((root, source.cache_key(), scope, ignore_patterns))
}

#[cfg(test)]
mod tests {
    use super::scope_entries::{
        visible_tree_oid_from_files, visible_tree_oid_from_files_if_scope_present,
    };
    use super::{GitObjectHashAlgorithm, StoredVisibleScopeOidResolver, TrackedFile};
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    #[test] // xpec: A8,m
    fn parent_scope_matches_non_utf8_child_entry() {
        let resolver = resolver_with_files(vec![TrackedFile {
            path: b"dir/nonutf8-\xff.txt".to_vec(),
            mode: "100644".to_string(),
            object_id: "0123456789012345678901234567890123456789".to_string(),
        }]);

        assert!(resolver
            .oid_for_stored_visible_scope(&["dir".to_string()])
            .is_ok());
    }

    #[test] // xpec: A8,m
    fn visible_scope_hash_accepts_gitlink_entries() {
        let resolver = resolver_with_files(vec![TrackedFile {
            path: b"deps/example".to_vec(),
            mode: "160000".to_string(),
            object_id: "0123456789012345678901234567890123456789".to_string(),
        }]);

        assert!(resolver
            .oid_for_stored_visible_scope(&["deps".to_string()])
            .is_ok());
    }

    #[test] // xpec: A8,m
    fn explicit_absent_stored_scope_hashes_the_empty_tree() {
        let resolver = resolver_with_files(vec![TrackedFile {
            path: b"src/check/run/run.rs".to_vec(),
            mode: "100644".to_string(),
            object_id: "0123456789012345678901234567890123456789".to_string(),
        }]);

        let absent_scope_oid = resolver
            .oid_for_stored_visible_scope(&["src/check/run.rs".to_string()])
            .unwrap();
        let empty_tree_oid = resolver_with_files(Vec::new())
            .oid_for_stored_visible_scope(&[".".to_string()])
            .unwrap();

        assert!(resolver
            .oid_for_stored_visible_scope(&["src/check/run".to_string()])
            .is_ok());
        assert_eq!(absent_scope_oid, empty_tree_oid);
    }

    #[test] // xpec: A8
    fn required_visible_scope_hash_accepts_a_stale_term_when_another_term_matches() {
        let files = vec![TrackedFile {
            path: b"src/check/config/validation.rs".to_vec(),
            mode: "100644".to_string(),
            object_id: "0123456789012345678901234567890123456789".to_string(),
        }];

        assert!(visible_tree_oid_from_files(
            &files,
            &[
                "src/check/config/validation.rs".to_string(),
                "src/check/run/selection/cooldown.rs".to_string(),
            ],
            GitObjectHashAlgorithm::Sha1,
        )
        .is_ok());
    }

    #[test] // xpec: r8
    fn optional_visible_scope_oid_uses_union_presence_semantics() {
        let files = vec![TrackedFile {
            path: b"src/check/config/validation.rs".to_vec(),
            mode: "100644".to_string(),
            object_id: "0123456789012345678901234567890123456789".to_string(),
        }];

        assert!(visible_tree_oid_from_files_if_scope_present(
            &files,
            &["src/check/config".to_string(), "missing".to_string()],
            GitObjectHashAlgorithm::Sha1,
        )
        .unwrap()
        .is_some());
        assert!(visible_tree_oid_from_files_if_scope_present(
            &files,
            &["missing".to_string()],
            GitObjectHashAlgorithm::Sha1,
        )
        .unwrap()
        .is_none());
    }

    #[test] // xpec: d
    fn stored_visible_scope_oid_is_reused_for_the_same_scope() {
        let mut resolver = resolver_with_files(vec![TrackedFile {
            path: b"file.txt".to_vec(),
            mode: "100644".to_string(),
            object_id: "0123456789012345678901234567890123456789".to_string(),
        }]);
        let scope = ["file.txt".to_string()];
        let first = resolver.oid_for_stored_visible_scope(&scope).unwrap();
        Rc::make_mut(&mut resolver.files)[0].object_id =
            "9876543210987654321098765432109876543210".to_string();

        let reused = resolver.oid_for_stored_visible_scope(&scope).unwrap();

        assert_eq!(reused, first);
    }

    fn resolver_with_files(files: Vec<TrackedFile>) -> StoredVisibleScopeOidResolver {
        StoredVisibleScopeOidResolver {
            files: Rc::new(files),
            object_hash_algorithm: GitObjectHashAlgorithm::Sha1,
            visible_tree_oids: Rc::new(RefCell::new(BTreeMap::new())),
        }
    }
}
