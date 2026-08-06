mod construction;
mod view;

use crate::check::ExpectationIdentity;
use crate::config_types::CheckConfig;
use crate::git::TreeSource;
use crate::materialization::TreeMaterializer;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

pub(crate) struct CheckRuntime<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a CheckConfig,
    pub(crate) expectation_identities: &'a [ExpectationIdentity],
    disable_session_isolation: bool,
    mode: CheckRuntimeMode<'a>,
}

enum CheckRuntimeMode<'a> {
    Materialized {
        tree_source: &'a TreeSource,
        tree_context: CheckTreeContext,
        tree_materializer: &'a TreeMaterializer,
        persistent_history: bool,
    },
    InPlaceCheck {
        persistent_status_history: bool,
    },
    InPlaceTemporaryQuery,
}

#[derive(Clone)]
pub(crate) struct CheckTreeContext {
    pub(crate) checked_tree_oid: String,
    pub(crate) against_tree_oid: String,
    pub(crate) head_tree_oid: Option<String>,
    pub(crate) explicit_diff_from_tree_oids: BTreeMap<String, String>,
    pub(crate) checked_file_count: usize,
    pub(crate) prompt_git_environment: Vec<(OsString, OsString)>,
}

impl CheckRuntime<'_> {
    pub(crate) fn disable_session_isolation(&self) -> bool {
        self.disable_session_isolation
    }

    pub(crate) fn is_in_place(&self) -> bool {
        matches!(
            self.mode,
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery
        )
    }

    pub(crate) fn evaluator_interrogations_never_hide_files(&self) -> bool {
        // In-place mode evaluates against the checked directory as-is. Its evaluator
        // interrogations never hide files through q-scope, expectation ignore,
        // scope narrowing, or retry behavior.
        self.is_in_place()
    }

    pub(crate) fn persistent_check_state_root(&self) -> Option<&Path> {
        match self.mode {
            CheckRuntimeMode::Materialized {
                persistent_history, ..
            } => persistent_history.then_some(self.root),
            CheckRuntimeMode::InPlaceCheck {
                persistent_status_history,
            } => persistent_status_history.then_some(self.root),
            CheckRuntimeMode::InPlaceTemporaryQuery => None,
        }
    }

    pub(crate) fn tree_source(&self) -> Option<&TreeSource> {
        self.git_context().map(|(tree_source, _)| tree_source)
    }

    pub(crate) fn git_checked_tree_oid(&self) -> Option<&str> {
        self.git_context()
            .map(|(_, context)| context.checked_tree_oid.as_str())
    }

    pub(crate) fn prompt_git_environment(&self) -> &[(OsString, OsString)] {
        self.git_context().map_or(&[], |(_, context)| {
            context.prompt_git_environment.as_slice()
        })
    }

    pub(crate) fn git_against_tree_oid(&self) -> Option<&str> {
        self.git_context()
            .map(|(_, context)| context.against_tree_oid.as_str())
    }

    pub(crate) fn git_head_tree_oid(&self) -> Option<&str> {
        self.git_context()
            .and_then(|(_, context)| context.head_tree_oid.as_deref())
    }

    pub(crate) fn explicit_diff_from_tree_oid(&self, diff_from: &str) -> Option<&str> {
        self.git_context().and_then(|(_, context)| {
            context
                .explicit_diff_from_tree_oids
                .get(diff_from)
                .map(String::as_str)
        })
    }

    pub(crate) fn set_explicit_diff_from_tree_oids(
        &mut self,
        resolved: BTreeMap<String, String>,
    ) -> Result<(), String> {
        match &mut self.mode {
            CheckRuntimeMode::Materialized { tree_context, .. } => {
                tree_context.explicit_diff_from_tree_oids = resolved;
                Ok(())
            }
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery => {
                Err("cannot prepare Git trees for an in-place check".to_string())
            }
        }
    }

    fn git_context(&self) -> Option<(&TreeSource, &CheckTreeContext)> {
        match &self.mode {
            CheckRuntimeMode::Materialized {
                tree_source,
                tree_context,
                ..
            } => Some((tree_source, tree_context)),
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_types::AgentConfig;
    use crate::git::VisibleTreeOidCache;
    use crate::hash::full_scope;
    use crate::repo_inspection::RepoInspectionCache;
    use std::path::PathBuf;

    fn test_config() -> CheckConfig {
        CheckConfig {
            version: 1,
            agent: AgentConfig::default(),
            expectations: Vec::new(),
        }
    }

    #[test] // xpec: 90
    fn in_place_runtime_uses_full_scope_and_checked_directory() {
        let root = PathBuf::from("/tmp/canon-in-place-runtime");
        let config = test_config();
        let runtime = CheckRuntime::in_place(&root, &config, true);
        let requested_scope = vec!["src".to_string()];

        assert!(runtime.tree_source().is_none());
        assert_eq!(runtime.persistent_check_state_root(), Some(root.as_path()));
        assert!(runtime.git_checked_tree_oid().is_none());
        assert!(runtime.git_against_tree_oid().is_none());
        assert_eq!(
            runtime
                .visible_scope(&AgentConfig::default(), &requested_scope)
                .unwrap(),
            full_scope()
        );
        assert_eq!(
            runtime.scope_without_reusable_q_scope_history().unwrap(),
            full_scope()
        );
        assert_eq!(
            runtime
                .session_root_for_scope(&AgentConfig::default(), &requested_scope, None)
                .unwrap(),
            root
        );
    }

    #[test] // xpec: 1g,90,g2
    fn in_place_runtime_without_state_namespace_keeps_no_persistent_history() {
        let root = PathBuf::from("/tmp/canon-in-place-without-state");
        let config = test_config();
        let runtime = CheckRuntime::in_place(&root, &config, false);

        assert!(runtime.is_in_place());
        assert!(runtime.persistent_check_state_root().is_none());
        assert_eq!(
            runtime.scope_without_reusable_q_scope_history().unwrap(),
            full_scope()
        );
    }

    #[test] // xpec: l
    fn in_place_temporary_query_runtime_has_no_state_root() {
        let root = PathBuf::from("/tmp/canon-in-place-query-runtime");
        let config = test_config();
        let runtime = CheckRuntime::in_place_temporary_query(&root, &config);

        assert!(runtime.is_in_place());
        assert!(runtime.persistent_check_state_root().is_none());
        assert_eq!(
            runtime.scope_without_reusable_q_scope_history().unwrap(),
            full_scope()
        );
    }

    #[test] // xpec: l
    fn materialized_runtime_without_persistent_history_has_no_state_root() {
        use std::fs;
        use std::process::{self, Command};
        use std::time::{SystemTime, UNIX_EPOCH};

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "canon-runtime-no-history-{}-{}",
            process::id(),
            stamp
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .output()
            .unwrap();
        fs::write(root.join("README.md"), "hello\n").unwrap();
        Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&root)
            .output()
            .unwrap();
        let config = test_config();
        let source = TreeSource::Staged;
        let tree_materializer = TreeMaterializer::apply_for_tree_source_with_repo_inspection_cache(
            &root,
            source.clone(),
            RepoInspectionCache::new(),
            &crate::platform::filesystem::PrivateTemporaryDirectoryAllocator::new(),
        )
        .unwrap();
        let runtime = CheckRuntime::materialized_without_persistent_history(
            &root,
            &tree_materializer,
            &source,
            CheckTreeContext {
                checked_tree_oid: "checked".to_string(),
                against_tree_oid: "against".to_string(),
                head_tree_oid: None,
                explicit_diff_from_tree_oids: Default::default(),
                checked_file_count: 1,
                prompt_git_environment: Vec::new(),
            },
            &config,
            false,
        );

        assert!(runtime.persistent_check_state_root().is_none());
        assert_eq!(
            runtime.scope_without_reusable_q_scope_history().unwrap(),
            full_scope()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: 90
    fn in_place_runtime_never_makes_requested_scope_smaller() {
        let root = PathBuf::from("/tmp/canon-in-place-runtime");
        let config = test_config();
        let runtime = CheckRuntime::in_place(&root, &config, true);
        let mut cache = VisibleTreeOidCache::new();
        let agent = AgentConfig::default();
        let src_scope = vec!["src".to_string()];

        assert_eq!(
            runtime
                .num_invisible_files(&mut cache, &agent, &full_scope())
                .unwrap(),
            0
        );
        assert_eq!(
            runtime
                .num_invisible_files(&mut cache, &agent, &src_scope)
                .unwrap(),
            0
        );
        assert_eq!(
            runtime.visible_scope(&agent, &src_scope).unwrap(),
            full_scope()
        );
        assert!(runtime
            .visible_tree_oid(&mut cache, &agent, &["missing".to_string()])
            .unwrap()
            .is_none());
    }
}
