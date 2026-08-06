use super::XpecStateCache;
use crate::check::{for_each_unique_report_record, CheckRunReport, ExpectationIdentity};
use crate::config_types::{CheckConfig, ExpectationTarget, ExpectationTo};
use crate::fs_util::{
    ensure_dir_without_symlinks, for_each_nonempty_line, reject_symlink,
    write_temp_file_then_replace,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

// [fh] The only cross-configuration failure history retains a fixed record
// count and is compacted to a constant multiple of that retained suffix.
const FAILURE_HISTORY_LIMIT: usize = 64;
const RECURRING_FAILURE_TAIL: usize = 2;
static FAILURE_HISTORY_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FailureHistoryRecord {
    head_tree_oid: String,
    // Full ID remains the canonical persistent expectation reference. The
    // short ID is separate historical output data because recurrence follows
    // the exact short ID printed by the run that produced this record.
    xpec_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    short_id: Option<String>,
    to: String,
    response_error: bool,
    target_is_diff: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diff_from_oid: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FailureHistoryFeedback {
    pub(crate) short_id: String,
    pub(crate) recurring: bool,
    pub(crate) diff_from_oid: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecurringFailure {
    diff_from_oid: Option<String>,
}

impl XpecStateCache {
    pub(crate) fn append_check_failure_history(
        &mut self,
        root: &Path,
        head_tree_oid: &str,
        config: &CheckConfig,
        identities: &[ExpectationIdentity],
        report: &CheckRunReport,
    ) -> Result<Option<FailureHistoryFeedback>, String> {
        // [fh] A normal run may update its bounded global failure history only
        // after this exact complete configuration has passed xpec retention.
        self.require_retained_configuration(root, identities)?;
        let path = self.failure_history_path(root)?;
        let mut history = read_failure_history(&path)?;
        retain_recent_failures(&mut history);
        maybe_compact_failure_history(&path, &history)?;
        let expectations = identities
            .iter()
            .zip(&config.expectations)
            .map(|(identity, expectation)| (identity.id.as_str(), expectation))
            .collect::<BTreeMap<_, _>>();
        let mut current_failure_count = 0usize;
        let mut sole_current_failure_identity = None;
        let mut new_failures = Vec::new();
        for_each_unique_report_record(&report.records, &report.cached_passes, |record| {
            if record.passed() {
                return;
            }
            current_failure_count += 1;
            sole_current_failure_identity = if current_failure_count == 1 {
                Some((record.id.clone(), record.display_id.clone()))
            } else {
                None
            };
            let expectation = expectations.get(record.id.as_str()).copied();
            new_failures.push(FailureHistoryRecord {
                head_tree_oid: head_tree_oid.to_string(),
                xpec_id: record.id.clone(),
                short_id: Some(record.display_id.clone()),
                to: record.to.as_str().to_string(),
                response_error: record.error.is_some(),
                target_is_diff: expectation.is_some_and(|expectation| {
                    matches!(expectation.target, Some(ExpectationTarget::Diff))
                }),
                diff_from_oid: record.diff_from_tree_oid.clone(),
            });
        });
        append_failure_history(&path, &new_failures)?;
        history.extend(new_failures);
        retain_recent_failures(&mut history);

        let feedback =
            if let Some((failed_xpec_id, failed_short_id)) = sole_current_failure_identity {
                // Full ID proves the persistent reference is the current
                // xpec; short ID preserves the pseudocode's exact public
                // identity assertion before the optional recurrence decision.
                // xpec: ex
                assert_eq!(
                    history
                        .last()
                        .map(|failure| (failure.xpec_id.as_str(), failure.short_id.as_deref())),
                    Some((failed_xpec_id.as_str(), Some(failed_short_id.as_str()))),
                    "the current failure record must be appended to fail history"
                );
                let recurring = recurring_failure(&history, head_tree_oid);
                Some(FailureHistoryFeedback {
                    short_id: failed_short_id,
                    recurring: recurring.is_some(),
                    diff_from_oid: recurring.and_then(|failure| failure.diff_from_oid),
                })
            } else {
                None
            };
        Ok(feedback)
    }

    fn failure_history_path(&mut self, root: &Path) -> Result<PathBuf, String> {
        let xpecs_dir = self.xpecs_dir(root)?;
        let state_root = xpecs_dir.parent().ok_or_else(|| {
            format!(
                "failed to resolve failure history parent for {}",
                xpecs_dir.display()
            )
        })?;
        Ok(state_root.join(crate::state_paths::FAILURE_HISTORY_FILE_NAME))
    }
}

fn recurring_failure(
    history: &[FailureHistoryRecord],
    head_tree_oid: &str,
) -> Option<RecurringFailure> {
    let mut last_short_ids = Vec::new();
    for failure in history.iter().rev() {
        if failure.head_tree_oid != head_tree_oid
            || failure.to != ExpectationTo::Agent.as_str()
            || failure.response_error
        {
            if last_short_ids.is_empty() {
                return None;
            }
            continue;
        }
        // A legacy record has no historical short ID to compare. Treat it as
        // an unknown boundary instead of manufacturing recurrence from its
        // full ID or the current configuration's possibly different prefix.
        let short_id = failure.short_id.as_deref()?;
        last_short_ids.push(short_id);
        if last_short_ids.len() == RECURRING_FAILURE_TAIL {
            break;
        }
    }
    if last_short_ids.len() != RECURRING_FAILURE_TAIL || last_short_ids[0] != last_short_ids[1] {
        return None;
    }
    let current = history.last()?;
    Some(RecurringFailure {
        diff_from_oid: if current.target_is_diff {
            current.diff_from_oid.clone()
        } else {
            None
        },
    })
}

fn read_failure_history(path: &Path) -> Result<Vec<FailureHistoryRecord>, String> {
    let mut history = Vec::new();
    for_each_nonempty_line(path, |line_number, line| {
        let record = serde_json::from_str(&line).map_err(|error| {
            format!(
                "failed to parse {} line {}: {}",
                path.display(),
                line_number,
                error
            )
        })?;
        history.push(record);
        Ok(())
    })?;
    Ok(history)
}

fn retain_recent_failures(history: &mut Vec<FailureHistoryRecord>) {
    let obsolete_count = history.len().saturating_sub(FAILURE_HISTORY_LIMIT);
    history.drain(..obsolete_count);
}

fn append_failure_history(
    path: &Path,
    new_failures: &[FailureHistoryRecord],
) -> Result<(), String> {
    if new_failures.is_empty() {
        return Ok(());
    }
    let content = render_failure_history(new_failures, path)?;
    if let Some(parent) = path.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    reject_symlink(path)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("failed to open {}: {}", path.display(), error))?;
    file.write_all(&content)
        .and_then(|()| file.flush())
        .map_err(|error| format!("failed to append {}: {}", path.display(), error))
}

fn maybe_compact_failure_history(
    path: &Path,
    retained_history: &[FailureHistoryRecord],
) -> Result<(), String> {
    reject_symlink(path)?;
    let file_size = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to inspect {}: {}", path.display(), error)),
    };
    let retained_content = render_failure_history(retained_history, path)?;
    // Rewrite only after obsolete append records are at least as large as the
    // retained suffix. The rewrite is therefore paid for by bytes appended
    // since the preceding compact form, while the file remains bounded by a
    // constant multiple of the retained 64-record history.
    if !failure_history_needs_compaction(file_size, retained_content.len()) {
        return Ok(());
    }
    let counter = FAILURE_HISTORY_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = path.with_extension(format!("jsonl.tmp-{}-{counter}", std::process::id()));
    write_temp_file_then_replace(&temp_path, path, |file| {
        file.write_all(&retained_content)
            .map_err(|error| format!("failed to write {}: {}", temp_path.display(), error))
    })
}

fn failure_history_needs_compaction(file_size: u64, retained_size: usize) -> bool {
    let retained_size = u64::try_from(retained_size).unwrap_or(u64::MAX);
    file_size > 0 && retained_size.saturating_mul(2) <= file_size
}

fn render_failure_history(
    history: &[FailureHistoryRecord],
    path: &Path,
) -> Result<Vec<u8>, String> {
    let mut content = Vec::new();
    for record in history {
        serde_json::to_writer(&mut content, record)
            .map_err(|error| format!("failed to write {}: {}", path.display(), error))?;
        content.push(b'\n');
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::{failure_history_needs_compaction, recurring_failure, FailureHistoryRecord};
    use crate::check::{
        load_check_config, CheckRecord, CheckResult, CheckRunReport, ExpectationIdentity,
    };
    use crate::config_types::{CheckConfig, ExpectationTo};
    use crate::git::TreeSource;
    use crate::repo_inspection::RepoInspectionCache;
    use crate::xpec_state::XpecStateCache;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn failure(xpec_id: &str, head_tree_oid: &str) -> FailureHistoryRecord {
        FailureHistoryRecord {
            head_tree_oid: head_tree_oid.to_string(),
            xpec_id: xpec_id.to_string(),
            short_id: Some(xpec_id.to_string()),
            to: "agent".to_string(),
            response_error: false,
            target_is_diff: false,
            diff_from_oid: None,
        }
    }

    #[test] // xpec: ex
    fn recurrence_skips_irrelevant_older_failures_after_the_current_failure() {
        let history = vec![
            failure("x", "head"),
            failure("other", "other-head"),
            failure("x", "head"),
        ];

        let recurring = recurring_failure(&history, "head").unwrap();

        assert_eq!(recurring.diff_from_oid, None);
    }

    #[test] // xpec: ex
    fn current_irrelevant_failure_prevents_a_recurrence_warning() {
        let mut current = failure("x", "head");
        current.response_error = true;
        let history = vec![failure("x", "head"), current];

        assert!(recurring_failure(&history, "head").is_none());
    }

    #[test] // xpec: ex
    fn recurrence_compares_historical_short_ids_not_full_ids() {
        let root = temporary_git_project();
        let config = failure_history_config(&root);
        let mut state = XpecStateCache::default();

        let first = append_public_failure(&mut state, &root, &config, "full-id-one");
        let second = append_public_failure(&mut state, &root, &config, "full-id-two");

        assert!(!first.recurring);
        assert!(second.recurring);
        assert_eq!(second.short_id, "same");
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: ex
    fn recurring_diff_failure_carries_its_resolved_diff_from_oid() {
        let mut previous = failure("x", "head");
        previous.target_is_diff = true;
        previous.diff_from_oid = Some("previous-base".to_string());
        let mut current = failure("x", "head");
        current.target_is_diff = true;
        current.diff_from_oid = Some("current-base".to_string());

        let recurring = recurring_failure(&[previous, current], "head").unwrap();

        assert_eq!(recurring.diff_from_oid.as_deref(), Some("current-base"));
    }

    #[test] // xpec: kL
    fn failure_history_rewrite_waits_until_obsolete_bytes_pay_for_it() {
        assert!(!failure_history_needs_compaction(199, 100));
        assert!(failure_history_needs_compaction(200, 100));
    }

    fn append_public_failure(
        state: &mut XpecStateCache,
        root: &Path,
        config: &CheckConfig,
        full_id: &str,
    ) -> super::FailureHistoryFeedback {
        let identity = ExpectationIdentity {
            id: full_id.to_string(),
            display_id: "same".to_string(),
        };
        state
            .retain_only_current_configuration(root, std::slice::from_ref(&identity))
            .unwrap();
        state
            .append_check_failure_history(
                root,
                "head",
                config,
                std::slice::from_ref(&identity),
                &CheckRunReport {
                    records: vec![failed_record(full_id)],
                    cached_passes: Vec::new(),
                    pending: 0,
                },
            )
            .unwrap()
            .unwrap()
    }

    fn failed_record(full_id: &str) -> CheckRecord {
        CheckRecord {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            result: CheckResult::Fail,
            to: ExpectationTo::Agent,
            question: Some("Does it pass?".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: "no".to_string(),
            error: None,
            evidence: Some("test evidence".to_string()),
            scope: vec![".".to_string()],
            q_scope_suggestion: None,
            visible_tree_oid: None,
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
            id: full_id.to_string(),
            display_id: "same".to_string(),
        }
    }

    fn failure_history_config(root: &Path) -> CheckConfig {
        let config_path = Path::new(".canon/check.yml");
        fs::create_dir_all(root.join(".canon")).unwrap();
        fs::write(
            root.join(config_path),
            "version: 1\npresets:\n  default: {}\nxpecs:\n  - q: \"Does it pass?\"\n    a: \"yes\"\n",
        )
        .unwrap();
        let output = Command::new("git")
            .args(["add", ".canon/check.yml"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(output.status.success());
        load_check_config(
            &mut RepoInspectionCache::new(),
            root,
            config_path,
            &TreeSource::Staged,
        )
        .unwrap()
    }

    fn temporary_git_project() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "canon-failure-history-component-{}-{unique}",
            process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let output = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(output.status.success());
        root
    }
}
