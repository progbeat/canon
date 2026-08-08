use crate::check::core::ResolvedExpectation;
use crate::config_types::AgentConfig;
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::hash::full_scope;
use crate::scope::{sanitize_scope, scope_is_within};
use crate::xpec_state::LastResult;
use crate::xpec_state::XpecStateCache;
use std::path::Path;

pub(crate) fn initial_q_scope_for_check_run(
    root: &Path,
    expectation: &ResolvedExpectation,
    xpec_state: &mut XpecStateCache,
) -> Result<Vec<String>, String> {
    if let Some(paths) = expectation.q_scope.paths() {
        return Ok(paths.to_vec());
    }
    let last_pass = xpec_state.read_last_pass(root, expectation)?;
    Ok(initial_auto_q_scope_from_last_pass(last_pass.as_ref()))
}

fn initial_auto_q_scope_from_last_pass(last_pass: Option<&LastResult>) -> Vec<String> {
    last_pass
        .filter(|result| !result.q_scope.is_empty())
        .map(|result| result.q_scope.clone())
        .unwrap_or_else(full_scope)
}

pub(crate) fn validated_narrower_q_scope_suggestion(
    root: &Path,
    source: &TreeSource,
    agent: &AgentConfig,
    suggestion: &[String],
    current_scope: &[String],
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Option<Vec<String>>, String> {
    let suggested_scope = match sanitize_scope(suggestion) {
        Ok(scope) => scope,
        Err(_) => return Ok(None),
    };
    if !scope_is_within(&suggested_scope, current_scope) {
        return Ok(None);
    }
    if !matches!(
        visible_tree_oid_cache.visible_tree_oid_for_reuse(root, source, agent, &suggested_scope,),
        Ok(Some(_))
    ) {
        return Ok(None);
    }
    let current_count =
        visible_tree_oid_cache.visible_file_count(root, source, agent, current_scope)?;
    let suggested_count =
        match visible_tree_oid_cache.visible_file_count(root, source, agent, &suggested_scope) {
            Ok(count) => count,
            Err(_) => return Ok(None),
        };
    if suggestion_is_at_least_25_percent_smaller(current_count, suggested_count) {
        Ok(Some(suggested_scope))
    } else {
        Ok(None)
    }
}

fn suggestion_is_at_least_25_percent_smaller(current_count: usize, suggested_count: usize) -> bool {
    suggested_count < current_count
        && suggested_count.saturating_mul(4) <= current_count.saturating_mul(3)
}

pub(crate) fn initial_q_scope_without_history(expectation: &ResolvedExpectation) -> Vec<String> {
    expectation
        .q_scope
        .paths()
        .map(<[String]>::to_vec)
        .unwrap_or_else(full_scope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xpec_state::{LastResultResponse, LastResultStatus};

    #[test] // xpec: UR
    fn auto_q_scope_uses_only_a_present_last_pass_q_scope() {
        let restricted_scope = vec!["src".to_string()];
        let pass = result(
            LastResultStatus::Pass,
            "2026-01-01T00:00:00Z",
            restricted_scope.clone(),
            LastResultResponse::answered("yes", "evidence", Some(vec!["ignored".to_string()])),
        );
        let in_place_pass = result(
            LastResultStatus::Pass,
            "2026-01-01T00:00:00Z",
            Vec::new(),
            LastResultResponse::answered("yes", "evidence", None),
        );

        assert_eq!(
            initial_auto_q_scope_from_last_pass(Some(&pass)),
            restricted_scope
        );
        assert_eq!(
            initial_auto_q_scope_from_last_pass(Some(&in_place_pass)),
            full_scope()
        );
        assert_eq!(initial_auto_q_scope_from_last_pass(None), full_scope());
    }

    fn result(
        status: LastResultStatus,
        response_timestamp: &str,
        q_scope: Vec<String>,
        response: LastResultResponse,
    ) -> LastResult {
        LastResult {
            response_timestamp: response_timestamp.to_string(),
            updated_timestamp: response_timestamp.to_string(),
            status,
            response,
            q_scope,
            visible_scope: Vec::new(),
            checked_tree_oid: None,
            visible_tree_oid: None,
            diff_from: None,
            diff_from_tree_oid: None,
        }
    }
}
