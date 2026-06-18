use super::ignore::{effective_ignore_exclusion_pathspecs, push_unique_pattern};
use super::normalize::sanitize_scope_paths;
use crate::config_types::AgentConfig;

pub(crate) fn visible_scope(
    agent: &AgentConfig,
    q_scope: &[String],
) -> Result<Vec<String>, String> {
    // The returned value is the complete visible-scope pathspec. Configured
    // ignores are not a second filter; they become excluding pathspec entries
    // inside the same pathspec later applied to the checked Git tree.
    let ignore_exclusions = effective_ignore_exclusion_pathspecs(agent)?;
    let base_scope = scope_without_configured_ignore_exclusions(q_scope, &ignore_exclusions);
    let mut scope = sanitize_scope_paths(&base_scope)?;
    for exclusion in ignore_exclusions {
        push_unique_pattern(&mut scope, exclusion);
    }
    Ok(scope)
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
