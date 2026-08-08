use super::{CheckRuntime, CheckRuntimeMode, CheckTreeContext};
use crate::check::ExpectationIdentity;
use crate::config_types::CheckConfig;
use crate::git::TreeSource;
use crate::materialization::TreeMaterializer;
use std::path::Path;

impl<'a> CheckRuntime<'a> {
    pub(crate) fn materialized(
        root: &'a Path,
        tree_materializer: &'a TreeMaterializer,
        tree_source: &'a TreeSource,
        tree_context: CheckTreeContext,
        config: &'a CheckConfig,
        disable_session_isolation: bool,
    ) -> CheckRuntime<'a> {
        Self::materialized_with_history(
            root,
            tree_materializer,
            tree_source,
            tree_context,
            config,
            disable_session_isolation,
            true,
        )
    }

    pub(crate) fn materialized_without_persistent_history(
        root: &'a Path,
        tree_materializer: &'a TreeMaterializer,
        tree_source: &'a TreeSource,
        tree_context: CheckTreeContext,
        config: &'a CheckConfig,
        disable_session_isolation: bool,
    ) -> CheckRuntime<'a> {
        Self::materialized_with_history(
            root,
            tree_materializer,
            tree_source,
            tree_context,
            config,
            disable_session_isolation,
            false,
        )
    }

    fn materialized_with_history(
        root: &'a Path,
        tree_materializer: &'a TreeMaterializer,
        tree_source: &'a TreeSource,
        tree_context: CheckTreeContext,
        config: &'a CheckConfig,
        disable_session_isolation: bool,
        persistent_history: bool,
    ) -> CheckRuntime<'a> {
        CheckRuntime {
            root,
            config,
            expectation_identities: &[],
            disable_session_isolation,
            mode: CheckRuntimeMode::Materialized {
                tree_source,
                tree_context,
                tree_materializer,
                persistent_history,
            },
        }
    }

    pub(crate) fn in_place(
        root: &'a Path,
        config: &'a CheckConfig,
        persistent_status_history: bool,
    ) -> CheckRuntime<'a> {
        // This runtime mode owns the in-place evaluator view: no materialized
        // Git tree, full-project visibility, and threads rooted at the
        // checked directory. Command execution queues every selected xpec
        // without cached-result reuse, reads and writes status-specific
        // last-result history only when a canonical persistent state namespace
        // exists. That history has no checkedTreeOid, so even an in-place pass
        // never defines the glossary's Git-tree checkpoint. Config validation
        // owns mode compatibility after raw config expansion.
        CheckRuntime {
            root,
            config,
            expectation_identities: &[],
            disable_session_isolation: true,
            mode: CheckRuntimeMode::InPlaceCheck {
                persistent_status_history,
            },
        }
    }

    pub(crate) fn in_place_temporary_query(
        root: &'a Path,
        config: &'a CheckConfig,
    ) -> CheckRuntime<'a> {
        CheckRuntime {
            root,
            config,
            expectation_identities: &[],
            disable_session_isolation: true,
            mode: CheckRuntimeMode::InPlaceTemporaryQuery,
        }
    }

    pub(crate) fn with_expectation_identities(
        mut self,
        identities: &'a [ExpectationIdentity],
    ) -> CheckRuntime<'a> {
        assert_eq!(identities.len(), self.config.expectations.len()); // xpec: d
        self.expectation_identities = identities;
        self
    }
}
