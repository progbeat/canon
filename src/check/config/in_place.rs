//! Applies only the separate `canon check --in-place` config contract.

use super::expansion::ExpandedCheckConfig;
use crate::config_types::CheckConfig;

// [Df] In-place evaluation has no Git state or cache. Its separate contract
// therefore rejects fields that require either one.
pub(crate) fn try_into_in_place_config(
    expanded: ExpandedCheckConfig,
) -> Result<CheckConfig, String> {
    let requirements = expanded.in_place_requirements;
    if requirements.config_uses_ignore {
        return Err(
            "configured `ignore` is invalid in in-place mode because path hiding requires Git"
                .to_string(),
        );
    }
    for expectation in requirements.git_backed_only_expectation_fields {
        if !expectation.git_backed_only_field_names.is_empty() {
            return Err(format!(
                "expectation {} is invalid in in-place mode: {}",
                expectation.item_number,
                expectation
                    .git_backed_only_field_names
                    .into_iter()
                    .map(|field| format!("`{field}` requires Git-backed check state"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    Ok(expanded.config)
}
