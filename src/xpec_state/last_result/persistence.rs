//! Cross-invocation file IO for one xpec's status-specific last-result history.
//!
//! Last-result files are bounded command outputs under XPECS_DIR, not
//! invocation-local working state. Configuration-wide cleanup precedes every
//! newly recorded result. Refreshing an existing same-tree pass cannot
//! introduce another xpec ID, so the fast gate may perform that one bounded
//! update without pruning state for a staged configuration it does not own.

use super::validation::{
    require_git_backed_diff_provenance, validate_git_fields_for_tree_context, validate_last_result,
};
use super::{last_result_status_for_record, LastResult, LastResultResponse, LastResultStatus};
use crate::check::{CheckRecord, ResolvedExpectation};
use crate::fs_util::{ensure_dir_without_symlinks, reject_symlink, write_temp_file_then_replace};
use crate::scope::visible_scope;
use crate::time::{format_record_timestamp, unix_timestamp};
use crate::xpec_state::XpecStateCache;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

static LAST_RESULT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl XpecStateCache {
    pub(crate) fn read_last_pass(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
    ) -> Result<Option<LastResult>, String> {
        self.read_last_result(root, expectation, LastResultStatus::Pass)
    }

    pub(crate) fn read_last_fail(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
    ) -> Result<Option<LastResult>, String> {
        self.read_last_result(root, expectation, LastResultStatus::Fail)
    }

    pub(crate) fn read_last_result(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
        status: LastResultStatus,
    ) -> Result<Option<LastResult>, String> {
        let id = expectation.require_configured_id()?.to_string();
        let key = (root.to_path_buf(), id, status);
        // [d] An invocation-local hit performs no filesystem access. On a miss,
        // read_last_result_path validates the path immediately before reading it.
        if let Some(cached) = self.last_results.get(&key) {
            return Ok(cached.clone());
        }
        let path = self.last_result_path(root, expectation, status)?;
        let result = read_last_result_path(&path, status)?;
        self.last_results.insert(key, result.clone());
        Ok(result)
    }

    pub(crate) fn write_last_result_for_record(
        &mut self,
        root: &Path,
        checked_tree_oid: Option<&str>,
        expectation: &ResolvedExpectation,
        record: &CheckRecord,
    ) -> Result<LastResult, String> {
        // [fh] The xpec-state component rejects every result write unless its
        // complete current configuration has already been exhaustively pruned.
        self.require_retained_expectation(root, expectation)?;
        self.write_last_result_for_record_inner(root, checked_tree_oid, expectation, record)
    }

    pub(crate) fn write_interrogation_last_result_for_record_or_absent_history(
        &mut self,
        root: Option<&Path>,
        checked_tree_oid: Option<&str>,
        expectation: &ResolvedExpectation,
        record: &CheckRecord,
    ) -> Result<Option<LastResult>, String> {
        // Normal Git-backed check interrogations render a prompt diff, so this
        // interrogation-only writer requires the resolved diff provenance.
        // Lower-level writers still accept absent provenance for refreshed or
        // synthetic records that did not come from such an interrogation.
        if root.is_some() && checked_tree_oid.is_some() {
            require_git_backed_diff_provenance(
                record.diff_from.as_deref(),
                record.diff_from_tree_oid.as_deref(),
            )?;
        }
        self.write_last_result_for_record_or_absent_history(
            root,
            checked_tree_oid,
            expectation,
            record,
        )
    }

    pub(crate) fn write_last_result_for_record_or_absent_history(
        &mut self,
        root: Option<&Path>,
        checked_tree_oid: Option<&str>,
        expectation: &ResolvedExpectation,
        record: &CheckRecord,
    ) -> Result<Option<LastResult>, String> {
        let Some(root) = root else {
            // Last Results are file-backed xpec state under XPECS_DIR. A
            // runtime with absent persistent history has no status-specific
            // files to update.
            return Ok(None);
        };
        self.write_last_result_for_record(root, checked_tree_oid, expectation, record)
            .map(Some)
    }

    fn write_last_result_for_record_inner(
        &mut self,
        root: &Path,
        checked_tree_oid: Option<&str>,
        expectation: &ResolvedExpectation,
        record: &CheckRecord,
    ) -> Result<LastResult, String> {
        let status = last_result_status_for_record(record);
        let now = format_record_timestamp(unix_timestamp()?);
        let q_scope = record.scope.clone();
        let visible_scope = checked_tree_oid
            .is_some()
            .then(|| visible_scope(&expectation.agent, &q_scope))
            .transpose()?;
        // [90] In-place results persist pass/fail history but have no Git diff
        // context. Reject forbidden provenance before normalization or any
        // state migration so validation can observe the caller's actual input.
        validate_git_fields_for_tree_context(
            checked_tree_oid,
            record.diff_from.as_deref(),
            record.diff_from_tree_oid.as_deref(),
            record.visible_tree_oid.as_deref(),
        )?;
        let (diff_from, diff_from_tree_oid) = if checked_tree_oid.is_some() {
            (record.diff_from.clone(), record.diff_from_tree_oid.clone())
        } else {
            (None, None)
        };
        let result = LastResult {
            response_timestamp: record.timestamp.clone(),
            updated_timestamp: now,
            status,
            response: LastResultResponse::from_record(record),
            q_scope: if checked_tree_oid.is_some() {
                q_scope.clone()
            } else {
                Vec::new()
            },
            visible_scope: visible_scope.unwrap_or_default(),
            checked_tree_oid: checked_tree_oid.map(str::to_string),
            visible_tree_oid: match (checked_tree_oid, status) {
                (Some(_), LastResultStatus::Pass) => Some(
                    record
                        .visible_tree_oid
                        .clone()
                        .ok_or("Git-backed pass record is missing visible tree OID".to_string())?,
                ),
                _ => None,
            },
            diff_from,
            diff_from_tree_oid,
        };
        if checked_tree_oid.is_none() {
            self.preserve_gate_results_before_in_place_update(root, expectation)?;
        }
        self.write_last_result(root, expectation, &result)?;
        if checked_tree_oid.is_some() {
            // [KD,cw] Gate's Git-backed cache is updated at the same component
            // boundary as the canonical result, never by an in-place writer.
            self.write_git_backed_last_result(root, expectation, &result)?;
        }
        Ok(result)
    }

    pub(in crate::xpec_state) fn refresh_existing_git_backed_pass_for_checked_tree(
        &mut self,
        root: &Path,
        current_checked_tree_oid: &str,
        expectation: &ResolvedExpectation,
    ) -> Result<LastResult, String> {
        // [fh,Ijl,KD] This operation starts from a persisted pass and cannot
        // create state for a previously absent xpec. Unlike a normal check,
        // gate reads the committed config while staged config may add IDs, so
        // gate must not run configuration-wide destructive retention.
        let mut refreshed = self
            .read_last_pass(root, expectation)?
            .ok_or_else(|| "cannot refresh an absent last-pass result".to_string())?;
        refreshed.updated_timestamp = format_record_timestamp(unix_timestamp()?);
        refreshed.checked_tree_oid = Some(current_checked_tree_oid.to_string());
        self.write_last_result(root, expectation, &refreshed)?;
        // [KD,cw] A reused Git-backed pass remains a known pass for gate even
        // if a later in-place check updates the canonical last-pass file.
        self.write_git_backed_last_result(root, expectation, &refreshed)?;
        Ok(refreshed)
    }

    fn write_last_result(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
        result: &LastResult,
    ) -> Result<(), String> {
        validate_last_result(result.status, result)?;
        let path = self.last_result_path(root, expectation, result.status)?;
        let temp_path = temp_path_for(&path)?;
        // [g2,Sh] Last-result files are whole-record cross-invocation history
        // snapshots. This writes one complete newly persisted status record
        // plus the `last.json` alias below; it never externalizes this
        // invocation's XpecStateCache or rewrites an accumulated state prefix.
        // For serialized snapshot payloads totaling N bytes, the primary files
        // total N and the alias adds at most one copy of each payload, so this
        // path writes at most 2N bytes.
        write_temp_file_then_replace(&temp_path, &path, |file| {
            serde_json::to_writer(&mut *file, result)
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
            std::io::Write::write_all(file, b"\n")
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))
        })?;
        refresh_last_json_link(&path, &self.xpec_dir(root, expectation)?.join("last.json"))?;
        let id = expectation.require_configured_id()?.to_string();
        self.last_results.insert(
            (root.to_path_buf(), id, result.status),
            Some(result.clone()),
        );
        Ok(())
    }

    fn last_result_path(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
        status: LastResultStatus,
    ) -> Result<PathBuf, String> {
        Ok(self.xpec_dir(root, expectation)?.join(status.file_name()))
    }
}

pub(in crate::xpec_state) enum LastResultPathState {
    Missing,
    Valid(Box<LastResult>),
    Invalid(String),
}

pub(in crate::xpec_state) fn inspect_last_result_path(
    path: &Path,
    expected_status: LastResultStatus,
) -> Result<LastResultPathState, String> {
    reject_symlink(path)?;
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok(LastResultPathState::Missing)
        }
        Err(err) => return Err(format!("failed to read {}: {}", path.display(), err)),
    };
    let result = match serde_json::from_str::<LastResult>(&content) {
        Ok(result) => result,
        Err(err) => return Ok(LastResultPathState::Invalid(err.to_string())),
    };
    if let Err(message) = validate_last_result(expected_status, &result) {
        return Ok(LastResultPathState::Invalid(message));
    }
    Ok(LastResultPathState::Valid(Box::new(result)))
}

fn read_last_result_path(
    path: &Path,
    expected_status: LastResultStatus,
) -> Result<Option<LastResult>, String> {
    match inspect_last_result_path(path, expected_status)? {
        LastResultPathState::Missing => Ok(None),
        LastResultPathState::Valid(result) => Ok(Some(*result)),
        LastResultPathState::Invalid(message) => Err(format!(
            "invalid last-result JSON in {}: {}",
            path.display(),
            message
        )),
    }
}

fn refresh_last_json_link(status_path: &Path, last_path: &Path) -> Result<(), String> {
    if let Some(parent) = last_path.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    reject_symlink(last_path)?;
    let temp_path = temp_path_for(last_path)?;
    match fs::hard_link(status_path, &temp_path) {
        Ok(()) => crate::fs_util::replace_file_with_temp(&temp_path, last_path),
        Err(hard_link_error) => {
            fs::copy(status_path, &temp_path).map_err(|copy_error| {
                let _ = fs::remove_file(&temp_path);
                format!(
                    "failed to hardlink {} to {}: {}; failed to copy instead: {}",
                    status_path.display(),
                    last_path.display(),
                    hard_link_error,
                    copy_error
                )
            })?;
            crate::fs_util::replace_file_with_temp(&temp_path, last_path)
        }
    }
}

fn temp_path_for(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("state path has no file name: {}", path.display()))?;
    let mut temp_name = file_name.to_os_string();
    let sequence = LAST_RESULT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    temp_name.push(format!(".tmp.{}.{}", process::id(), sequence));
    Ok(path.with_file_name(temp_name))
}
