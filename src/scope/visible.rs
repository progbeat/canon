use super::ignore::{effective_ignore_exclusion_pathspecs, push_unique_pattern};
use super::normalize::sanitize_scope_paths;
use super::pathspec::pathspec_is_exclude;
use crate::config_types::AgentConfig;

pub(crate) fn visible_scope(
    agent: &AgentConfig,
    q_scope: &[String],
) -> Result<Vec<String>, String> {
    // The returned value is the complete visible-scope pathspec. Configured
    // ignores become ordinary excluding pathspec entries inside that scope, so
    // later visible-tree selection still applies one pathspec to the checked
    // Git tree.
    let ignore_exclusions = effective_ignore_exclusion_pathspecs(agent)?;
    let base_scope = scope_without_configured_ignore_exclusions(q_scope, &ignore_exclusions);
    let mut scope = sanitize_scope_paths(&base_scope)?;
    for exclusion in ignore_exclusions {
        push_unique_pattern(&mut scope, exclusion);
    }
    Ok(scope)
}

pub(crate) fn q_scope_from_visible_scope(
    agent: &AgentConfig,
    visible_scope: &[String],
) -> Result<Vec<String>, String> {
    let ignore_exclusions = effective_ignore_exclusion_pathspecs(agent)?;
    let q_scope = scope_without_configured_ignore_exclusions(visible_scope, &ignore_exclusions);
    reject_reconstructed_exclusions(&q_scope)?;
    sanitize_scope_paths(&q_scope)
}

fn scope_without_configured_ignore_exclusions(
    scope: &[String],
    ignore_exclusions: &[String],
) -> Vec<String> {
    scope
        .iter()
        .filter(|pathspec| {
            !ignore_exclusions
                .iter()
                .any(|ignore| ignore == pathspec.as_str())
        })
        .cloned()
        .collect()
}

fn reject_reconstructed_exclusions(scope: &[String]) -> Result<(), String> {
    for pathspec in scope {
        if pathspec_is_exclude(pathspec)? {
            return Err(format!(
                "visible scope contains exclusion that is not configured for the current agent: {}",
                pathspec
            ));
        }
    }
    Ok(())
}
