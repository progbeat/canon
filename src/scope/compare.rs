use super::normalize::{normalize_scope_for_comparison, normalized_scope_contains};

pub(crate) fn scope_is_within(proposed: &[String], current: &[String]) -> bool {
    let Some(proposed) = normalize_scope_for_comparison(proposed) else {
        return false;
    };
    let Some(current) = normalize_scope_for_comparison(current) else {
        return false;
    };
    proposed.iter().all(|path| {
        current
            .iter()
            .any(|base| normalized_scope_contains(base, path))
    })
}
