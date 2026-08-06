use super::{
    cached_value, git_object_hash_algorithm, path_bytes_in_scope, scope_entry_is_tree,
    source_scope_cache_key, visible_scope, visible_scope_entries_from_files,
    visible_tree_oid_from_files, visible_tree_oid_from_files_if_scope_present,
    GitObjectHashAlgorithm, SourceScopeCacheKey, StoredVisibleScopeOidResolver, TrackedFile,
    TreeSource, VisibleTreeOidCache,
};
use crate::config_types::AgentConfig;
use crate::repo_inspection::RepoInspectionCache;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

impl VisibleTreeOidCache {
    pub(crate) fn new() -> VisibleTreeOidCache {
        VisibleTreeOidCache::with_repo_inspection_cache(RepoInspectionCache::new())
    }

    pub(crate) fn with_repo_inspection_cache(
        repo_inspection: RepoInspectionCache,
    ) -> VisibleTreeOidCache {
        VisibleTreeOidCache {
            required_visible_tree_oids: Rc::new(RefCell::new(BTreeMap::new())),
            visible_tree_oids: Rc::new(RefCell::new(BTreeMap::new())),
            visible_scope_entries: Rc::new(RefCell::new(BTreeMap::new())),
            stored_visible_scope_resolvers: Rc::new(RefCell::new(BTreeMap::new())),
            repo_inspection,
            object_hash_algorithms: Rc::new(RefCell::new(BTreeMap::new())),
        }
    }

    pub(crate) fn visible_tree_oid(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<String, String> {
        let visible_scope_pathspec = visible_scope(agent, scope)?;
        self.required_visible_tree_oid_for_source_visible_scope(
            root,
            source,
            agent,
            visible_scope_pathspec,
        )
    }

    pub(crate) fn visible_tree_oid_for_reuse(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Option<String>, String> {
        let visible_scope_pathspec = visible_scope(agent, scope)?;
        self.visible_tree_oid_for_source_visible_scope(root, source, agent, visible_scope_pathspec)
    }

    pub(crate) fn stored_visible_scope_oid_resolver(
        &mut self,
        root: &Path,
        source: &TreeSource,
    ) -> Result<StoredVisibleScopeOidResolver, String> {
        let key = (root.to_path_buf(), source.cache_key());
        if let Some(resolver) = self
            .stored_visible_scope_resolvers
            .borrow()
            .get(&key)
            .cloned()
        {
            return Ok(resolver);
        }
        // Same-tree lookup can scan many records with many stored visible
        // scopes. The shared resolver snapshots the source files once and
        // memoizes each stored scope. It deliberately has no AgentConfig or
        // q-scope input, so current ignore settings cannot alter a persisted
        // visibleScope.
        let resolver = StoredVisibleScopeOidResolver {
            files: Rc::new(self.files_for_source(root, source)?),
            object_hash_algorithm: self.object_hash_algorithm(root)?,
            visible_tree_oids: Rc::new(RefCell::new(BTreeMap::new())),
        };
        self.stored_visible_scope_resolvers
            .borrow_mut()
            .insert(key, resolver.clone());
        Ok(resolver)
    }

    pub(crate) fn checked_file_count(
        &mut self,
        root: &Path,
        source: &TreeSource,
    ) -> Result<usize, String> {
        self.files_for_source(root, source).map(|files| files.len())
    }

    pub(crate) fn visible_file_count(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<usize, String> {
        let visible_scope_pathspec = visible_scope(agent, scope)?;
        // This count is the other side of the prompt's
        // `num_invisible_files = checked_file_count - visible_file_count`.
        // The entries are selected solely by the complete visible-scope
        // pathspec against the checked Git tree; no token or relevance
        // heuristic removes files here.
        let entries =
            self.visible_scope_entries_for_source(root, source, agent, &visible_scope_pathspec)?;
        Ok(entries
            .iter()
            .filter(|entry| !scope_entry_is_tree(entry))
            .count())
    }

    pub(crate) fn visible_scope_intersects_pathspecs(
        &mut self,
        root: &Path,
        source: &TreeSource,
        visible_scope: &[String],
        pathspecs: &[String],
    ) -> Result<bool, String> {
        // Used by `canon show -- <pathspec>`. If every tracked file matched by
        // `pathspecs` were changed, the visible tree OID would change exactly
        // when at least one changed tracked file is selected by `visible_scope`.
        // Let Git select the changed-file set so command pathspecs retain Git's
        // ordinary wildcard and magic semantics. Visible scopes continue to use
        // canon's scope matcher because their non-magic terms are literal.
        for file in self.files_for_pathspecs(root, source, pathspecs)? {
            if path_bytes_in_scope(&file.path, visible_scope)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn git_oid_abbreviation(
        &mut self,
        root: &Path,
        oid: &str,
    ) -> Result<String, String> {
        self.repo_inspection.git_oid_abbreviation(root, oid)
    }

    pub(crate) fn git_tree_object_exists(
        &mut self,
        root: &Path,
        oid: &str,
    ) -> Result<bool, String> {
        self.repo_inspection.git_tree_object_exists(root, oid)
    }

    fn visible_tree_oid_for_source_visible_scope(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        visible_scope_pathspec: Vec<String>,
    ) -> Result<Option<String>, String> {
        let cache = Rc::clone(&self.visible_tree_oids);
        self.cached_visible_tree_oid_for_source_visible_scope(
            cache,
            root,
            source,
            agent,
            &visible_scope_pathspec,
            |files, object_hash_algorithm| {
                visible_tree_oid_from_files_if_scope_present(
                    files,
                    &visible_scope_pathspec,
                    object_hash_algorithm,
                )
            },
        )
    }

    fn cached_visible_tree_oid_for_source_visible_scope<T: Clone>(
        &mut self,
        cache: Rc<RefCell<BTreeMap<SourceScopeCacheKey, T>>>,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        visible_scope_pathspec: &[String],
        compute: impl FnOnce(&[TrackedFile], GitObjectHashAlgorithm) -> Result<T, String>,
    ) -> Result<T, String> {
        let key = source_scope_cache_key(root, source, agent, visible_scope_pathspec)?;
        cached_value(&cache, key, || {
            let files = self.files_for_source(root, source)?;
            let object_hash_algorithm = self.object_hash_algorithm(root)?;
            compute(&files, object_hash_algorithm)
        })
    }

    fn required_visible_tree_oid_for_source_visible_scope(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        visible_scope_pathspec: Vec<String>,
    ) -> Result<String, String> {
        let cache = Rc::clone(&self.required_visible_tree_oids);
        // An active interrogation applies its complete visible pathspec to the
        // current tree. A persisted q-scope can retain a path that was renamed;
        // the remaining include terms still select their ordinary Git-pathspec
        // union.
        self.cached_visible_tree_oid_for_source_visible_scope(
            cache,
            root,
            source,
            agent,
            &visible_scope_pathspec,
            |files, object_hash_algorithm| {
                visible_tree_oid_from_files(files, &visible_scope_pathspec, object_hash_algorithm)
            },
        )
    }

    fn visible_scope_entries_for_source(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Vec<String>, String> {
        let cache = Rc::clone(&self.visible_scope_entries);
        let key = source_scope_cache_key(root, source, agent, scope)?;
        cached_value(&cache, key, || {
            // Keep direct git subprocesses independent of the number of
            // scopes or history records: list each tree source once, then
            // filter scopes here.
            let files = self.files_for_source(root, source)?;
            visible_scope_entries_from_files(&files, scope)
        })
    }

    fn files_for_source(
        &mut self,
        root: &Path,
        source: &TreeSource,
    ) -> Result<Vec<TrackedFile>, String> {
        self.repo_inspection.git_tracked_files(root, source)
    }

    fn files_for_pathspecs(
        &mut self,
        root: &Path,
        source: &TreeSource,
        pathspecs: &[String],
    ) -> Result<Vec<TrackedFile>, String> {
        self.repo_inspection
            .git_tracked_files_for_pathspecs(root, source, pathspecs)
    }

    fn object_hash_algorithm(&mut self, root: &Path) -> Result<GitObjectHashAlgorithm, String> {
        cached_value(&self.object_hash_algorithms, root.to_path_buf(), || {
            git_object_hash_algorithm(root)
        })
    }
}

impl Default for VisibleTreeOidCache {
    fn default() -> Self {
        Self::new()
    }
}
