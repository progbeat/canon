use super::ignore::{effective_ignore_exclusion_pathspecs, push_unique_pattern};
use super::normalize::sanitize_scope_paths;
use crate::config_types::AgentConfig;

pub(crate) fn visible_scope(
    agent: &AgentConfig,
    q_scope: &[String],
) -> Result<Vec<String>, String> {
    let ignore_exclusions = effective_ignore_exclusion_pathspecs(agent)?;
    let base_scope = q_scope_without_configured_ignore_exclusions(q_scope, &ignore_exclusions);
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
    let q_scope = q_scope_without_configured_ignore_exclusions(visible_scope, &ignore_exclusions);
    sanitize_scope_paths(&q_scope)
}

fn q_scope_without_configured_ignore_exclusions(
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

#[cfg(test)]
mod tests {
    use super::visible_scope;
    use crate::config_types::AgentConfig;
    use crate::hash::full_scope;
    use crate::scope::path_bytes_in_scope;

    #[test]
    fn visible_scope_applies_configured_ignores_as_pathspec_exclusions() {
        let agent = AgentConfig {
            models: Vec::new(),
            thinking: "medium".to_string(),
            ignore: vec![".canon/**".to_string()],
            plugins: Vec::new(),
        };
        let scope = visible_scope(&agent, &full_scope()).unwrap();

        assert_eq!(
            scope,
            vec![".".to_string(), ":(exclude,glob).canon/**".to_string()]
        );
        assert!(!path_bytes_in_scope(b".canon/TODOs.md", &scope).unwrap());
        assert!(path_bytes_in_scope(b"src/main.rs", &scope).unwrap());
    }

    #[test]
    fn visible_scope_does_not_duplicate_existing_configured_exclusions() {
        let agent = AgentConfig {
            models: Vec::new(),
            thinking: "medium".to_string(),
            ignore: vec![".canon/**".to_string()],
            plugins: Vec::new(),
        };

        let scope = visible_scope(
            &agent,
            &["src".to_string(), ":(exclude,glob).canon/**".to_string()],
        )
        .unwrap();

        assert_eq!(
            scope,
            vec!["src".to_string(), ":(exclude,glob).canon/**".to_string()]
        );
    }
}
