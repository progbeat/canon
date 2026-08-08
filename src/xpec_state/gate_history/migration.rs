use super::super::last_result::{
    inspect_last_result_path, validate_last_result, LastResultPathState,
};
use super::super::{LastResult, LastResultStatus};
use super::model::{GitBackedFail, GitBackedPass};
use super::persistence::{load_history_file, persist_cache_path, CACHE_FILE_NAME};
use std::path::Path;

pub(in crate::xpec_state) fn preserve_canonical_results(xpec_dir: &Path) -> Result<(), String> {
    let cache_path = xpec_dir.join(CACHE_FILE_NAME);
    let existing = load_history_file(&cache_path)?;
    let mut cache = existing.clone().unwrap_or_default();
    if let Some(last_pass) = migratable_git_backed_last_result(
        xpec_dir,
        LastResultStatus::Pass,
        git_backed_pass_from_last_result,
    )? {
        cache.last_pass = Some(last_pass);
    }
    if let Some(last_fail) = migratable_git_backed_last_result(
        xpec_dir,
        LastResultStatus::Fail,
        git_backed_fail_from_last_result,
    )? {
        cache.last_fail = Some(last_fail);
    }
    if existing.as_ref() == Some(&cache) || (cache.last_pass.is_none() && cache.last_fail.is_none())
    {
        return Ok(());
    }
    persist_cache_path(&cache_path, &cache)
}

fn migratable_git_backed_last_result<T>(
    xpec_dir: &Path,
    status: LastResultStatus,
    convert: impl FnOnce(LastResult) -> Option<T>,
) -> Result<Option<T>, String> {
    match inspect_last_result_path(&xpec_dir.join(status.file_name()), status)? {
        LastResultPathState::Valid(result) => Ok(convert(*result)),
        LastResultPathState::Missing | LastResultPathState::Invalid(_) => Ok(None),
    }
}

pub(super) fn git_backed_pass_from_last_result(result: LastResult) -> Option<GitBackedPass> {
    result.checked_tree_oid.as_ref()?;
    Some(GitBackedPass {
        response_timestamp: result.response_timestamp,
        visible_scope: result.visible_scope,
        visible_tree_oid: result.visible_tree_oid?,
    })
}

pub(super) fn git_backed_fail_from_last_result(result: LastResult) -> Option<GitBackedFail> {
    Some(GitBackedFail {
        checked_tree_oid: result.checked_tree_oid?,
    })
}

pub(super) fn validate_git_backed_result(result: &LastResult) -> Result<(), String> {
    validate_last_result(result.status, result)?;
    if result.checked_tree_oid.is_none() {
        return Err("Git-backed result cache entry must contain checkedTreeOid".to_string());
    }
    Ok(())
}
