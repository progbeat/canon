use crate::git::{GitPromptObjectArtifacts, TreeSource};
use crate::platform::filesystem::PrivateTemporaryDirectoryAllocator;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub(crate) struct GitBackedCheckResources {
    pub(super) kind: GitBackedCheckResourceKind,
    tree_oid_cache: Rc<RefCell<BTreeMap<(PathBuf, TreeSource), String>>>,
}

pub(super) enum GitBackedCheckResourceKind {
    Persistent,
    TemporaryQuery(GitPromptObjectArtifacts),
}

impl GitBackedCheckResources {
    pub(crate) fn persistent() -> GitBackedCheckResources {
        Self::new(GitBackedCheckResourceKind::Persistent)
    }

    pub(crate) fn share_persistent(&self) -> GitBackedCheckResources {
        assert!(
            matches!(self.kind, GitBackedCheckResourceKind::Persistent),
            "only persistent check resources may span command stages"
        ); // xpec: d
        GitBackedCheckResources {
            kind: GitBackedCheckResourceKind::Persistent,
            tree_oid_cache: Rc::clone(&self.tree_oid_cache),
        }
    }

    pub(crate) fn temporary_query(
        root: &Path,
        temporary_directory_allocator: &PrivateTemporaryDirectoryAllocator,
    ) -> Result<GitBackedCheckResources, String> {
        // [3a,g2,l] These temporary Git objects are prompt input required to
        // render a diff. They do not persist query or command state.
        GitPromptObjectArtifacts::new(root, temporary_directory_allocator)
            .map(|artifacts| Self::new(GitBackedCheckResourceKind::TemporaryQuery(artifacts)))
    }

    fn new(kind: GitBackedCheckResourceKind) -> GitBackedCheckResources {
        GitBackedCheckResources {
            kind,
            tree_oid_cache: Rc::new(RefCell::new(BTreeMap::new())),
        }
    }

    pub(crate) fn tree_oid_for_prompt_diff(
        &self,
        root: &Path,
        source: &TreeSource,
    ) -> Result<String, String> {
        // This is the owning check or temporary-query invocation's sole
        // full-tree OID resolution boundary. Persistent checks clone-share the
        // memo across command stages; a temporary query keeps its memo in its
        // single owned resource. Both reuse later requests for the same
        // resolved content source instead of resolving the tree again.
        let cache_key = (root.to_path_buf(), source.clone());
        if let Some(tree_oid) = self.tree_oid_cache.borrow().get(&cache_key) {
            return Ok(tree_oid.clone());
        }
        let tree_oid = match &self.kind {
            GitBackedCheckResourceKind::Persistent => source.tree_oid_for_prompt_diff(root),
            GitBackedCheckResourceKind::TemporaryQuery(artifacts) => {
                source.tree_oid_for_temporary_prompt_diff(root, artifacts)
            }
        }?;
        self.tree_oid_cache
            .borrow_mut()
            .insert(cache_key, tree_oid.clone());
        Ok(tree_oid)
    }

    // [Tv] Resolve a parsed symbolic tree at the command's preparation
    // boundary. Persistent checks use the repository ODB; temporary queries
    // retain the private ODB environment that owns their prompt-only tree.
    pub(crate) fn freeze_tree_source(
        &self,
        root: &Path,
        source: TreeSource,
    ) -> Result<TreeSource, String> {
        if !matches!(source, TreeSource::Staged) {
            return Ok(source);
        }
        let tree_oid = self.tree_oid_for_prompt_diff(root, &source)?;
        Ok(match &self.kind {
            GitBackedCheckResourceKind::Persistent => TreeSource::Git { tree_oid },
            GitBackedCheckResourceKind::TemporaryQuery(artifacts) => TreeSource::TemporaryGit {
                tree_oid,
                environment: artifacts.prompt_environment(),
            },
        })
    }

    pub(super) fn prompt_git_environment(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        match &self.kind {
            GitBackedCheckResourceKind::Persistent => Vec::new(),
            GitBackedCheckResourceKind::TemporaryQuery(artifacts) => artifacts.prompt_environment(),
        }
    }

    pub(super) fn persists_failure_history(&self) -> bool {
        matches!(self.kind, GitBackedCheckResourceKind::Persistent)
    }
}
