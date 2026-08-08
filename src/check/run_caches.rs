use crate::git::VisibleTreeOidCache;
use crate::platform::filesystem::PrivateTemporaryDirectoryAllocator;
use crate::repo_inspection::RepoInspectionCache;
use crate::xpec_state::XpecStateCache;

// [d] One bundle spans the complete high-level check operation: command
// preparation, expectation inspection, selection, and evaluator interrogation
// share repository/file inspections and derived visible-tree hashes instead of
// recomputing them in phase-local caches. Every field is invocation-owned;
// `xpec_state` memoizes access to persisted records but owns no persistent
// retention. Parser-specific reuse stays at its owning boundary:
// CheckConfigExpansionCache memoizes parsed configuration expansion and
// InvocationResponseParseMemo memoizes evaluator-response parsing.
pub(crate) struct CheckRunCaches {
    pub(crate) xpec_state: XpecStateCache,
    pub(crate) visible_tree_oid_cache: VisibleTreeOidCache,
    pub(crate) repo_inspection: RepoInspectionCache,
    pub(crate) temporary_directory_allocator: PrivateTemporaryDirectoryAllocator,
}

impl CheckRunCaches {
    pub(crate) fn new() -> CheckRunCaches {
        CheckRunCaches::with_repo_inspection_cache(RepoInspectionCache::new())
    }

    pub(crate) fn with_repo_inspection_cache(
        repo_inspection: RepoInspectionCache,
    ) -> CheckRunCaches {
        CheckRunCaches {
            xpec_state: XpecStateCache::default(),
            visible_tree_oid_cache: VisibleTreeOidCache::with_repo_inspection_cache(
                repo_inspection.clone(),
            ),
            repo_inspection,
            temporary_directory_allocator: PrivateTemporaryDirectoryAllocator::new(),
        }
    }
}
