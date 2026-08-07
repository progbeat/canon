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

// [fh] The global chronological failure history retains a fixed record count
// and is compacted to a constant multiple of that retained suffix.
const FAILURE_HISTORY_LIMIT: usize = 64;
const FAILURE_HISTORY_COMPACTION_RECORD_LIMIT: usize = FAILURE_HISTORY_LIMIT * 2;
const REPEATED_XPEC_FAILURE_TAIL: usize = 2;
static FAILURE_HISTORY_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
/// One durable event in the bounded global history shared across `canon check`
/// invocations. It stores only facts used to compare failures between runs;
/// counters, pending work, and feedback for the current invocation stay in
/// memory and are never part of this record. [g2,ex,fh]
struct FailureHistoryRecord {
    head_tree_oid: String,
    // [L] The persistent expectation reference is always the full ID.
    xpec_id: String,
    to: String,
    response_error: bool,
    target_is_diff: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diff_from_oid: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FailureHistoryFeedback {
    pub(crate) short_id: String,
    pub(crate) repeated_xpec_failure: bool,
    pub(crate) diff_from_oid: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RepeatedXpecFailure {
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
        let expectations = identities
            .iter()
            .zip(&config.expectations)
            .map(|(identity, expectation)| (identity.id.as_str(), expectation))
            .collect::<BTreeMap<_, _>>();
        let current_display_ids = identities
            .iter()
            .map(|identity| (identity.id.as_str(), identity.display_id.as_str()))
            .collect::<BTreeMap<_, _>>();
        let mut history = read_failure_history(&path)?;
        let persisted_record_count = history.len();
        retain_recent_failures(&mut history);
        maybe_compact_failure_history(&path, &history, persisted_record_count)?;
        let mut current_failure_count = 0usize;
        let mut sole_current_failure_identity = None;
        let mut persistent_records_from_this_run = Vec::new();
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
            persistent_records_from_this_run.push(FailureHistoryRecord {
                head_tree_oid: head_tree_oid.to_string(),
                xpec_id: record.id.clone(),
                to: record.to.as_str().to_string(),
                response_error: record.error.is_some(),
                target_is_diff: expectation.is_some_and(|expectation| {
                    matches!(expectation.target, Some(ExpectationTarget::Diff))
                }),
                diff_from_oid: record.diff_from_tree_oid.clone(),
            });
        });
        // [ex,fh] One run cannot append more than the complete retained tail.
        retain_recent_failures(&mut persistent_records_from_this_run);
        // [g2,ex,fh] A run produces these durable historical facts, but their
        // purpose and lifetime are explicitly cross-run. Invocation-local
        // control state above remains in memory.
        append_failure_history(&path, &persistent_records_from_this_run)?;
        history.extend(persistent_records_from_this_run);
        retain_recent_failures(&mut history);

        let feedback =
            if let Some((failed_xpec_id, failed_short_id)) = sole_current_failure_identity {
                // [L,ex] Persistent history identifies the current xpec by
                // full ID, while recurrence uses its current public short ID.
                // xpec: L,ex
                assert_eq!(
                    history.last().and_then(|failure| {
                        current_display_ids.get(failure.xpec_id.as_str()).copied()
                    }),
                    Some(failed_short_id.as_str()),
                    "the current failure short ID must be appended to fail history"
                );
                // The full ID proves the appended record is the current xpec
                // even when another configuration once used the same prefix.
                // xpec: L,ex
                debug_assert_eq!(
                    history.last().map(|failure| failure.xpec_id.as_str()),
                    Some(failed_xpec_id.as_str())
                );
                let repeated_failure =
                    repeated_xpec_failure(&history, head_tree_oid, &current_display_ids);
                Some(FailureHistoryFeedback {
                    short_id: failed_short_id,
                    repeated_xpec_failure: repeated_failure.is_some(),
                    diff_from_oid: repeated_failure.and_then(|failure| failure.diff_from_oid),
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

fn repeated_xpec_failure(
    history: &[FailureHistoryRecord],
    head_tree_oid: &str,
    current_display_ids: &BTreeMap<&str, &str>,
) -> Option<RepeatedXpecFailure> {
    // [2Z,ex,gN,L] Resolve public IDs against the current collected set. An
    // uncollected record falls back to its full ID, which cannot equal a
    // collected xpec's shorter unique prefix. Equal resolved IDs therefore
    // refer to the same full xpec without making history configuration-local.
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
        let short_id = current_display_ids
            .get(failure.xpec_id.as_str())
            .copied()
            .unwrap_or(failure.xpec_id.as_str());
        last_short_ids.push(short_id.to_string());
        if last_short_ids.len() == REPEATED_XPEC_FAILURE_TAIL {
            break;
        }
    }
    if last_short_ids.len() != REPEATED_XPEC_FAILURE_TAIL || last_short_ids[0] != last_short_ids[1]
    {
        return None;
    }
    let current = history.last()?;
    Some(RepeatedXpecFailure {
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
        let record: FailureHistoryRecord = serde_json::from_str(&line).map_err(|error| {
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
    persisted_record_count: usize,
) -> Result<(), String> {
    reject_symlink(path)?;
    let file_size = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to inspect {}: {}", path.display(), error)),
    };
    let retained_content = render_failure_history(retained_history, path)?;
    // [ex,fh] Independent count and byte thresholds keep append-only history
    // bounded without changing its global chronological meaning.
    if !failure_history_needs_compaction(file_size, retained_content.len(), persisted_record_count)
    {
        return Ok(());
    }
    let counter = FAILURE_HISTORY_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = path.with_extension(format!("jsonl.tmp-{}-{counter}", std::process::id()));
    write_temp_file_then_replace(&temp_path, path, |file| {
        file.write_all(&retained_content)
            .map_err(|error| format!("failed to write {}: {}", temp_path.display(), error))
    })
}

fn failure_history_needs_compaction(
    file_size: u64,
    retained_size: usize,
    persisted_record_count: usize,
) -> bool {
    let retained_size = u64::try_from(retained_size).unwrap_or(u64::MAX);
    persisted_record_count >= FAILURE_HISTORY_COMPACTION_RECORD_LIMIT
        || (file_size > 0 && retained_size.saturating_mul(2) <= file_size)
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
    use super::{failure_history_needs_compaction, repeated_xpec_failure, FailureHistoryRecord};
    use crate::check::{
        load_check_config, CheckRecord, CheckResult, CheckRunReport, ExpectationIdentity,
    };
    use crate::config_types::{CheckConfig, ExpectationTo};
    use crate::git::TreeSource;
    use crate::repo_inspection::RepoInspectionCache;
    use crate::xpec_state::XpecStateCache;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn failure(xpec_id: &str, head_tree_oid: &str) -> FailureHistoryRecord {
        FailureHistoryRecord {
            head_tree_oid: head_tree_oid.to_string(),
            xpec_id: xpec_id.to_string(),
            to: "agent".to_string(),
            response_error: false,
            target_is_diff: false,
            diff_from_oid: None,
        }
    }

    #[test] // xpec: ex
    fn repeated_xpec_failure_skips_irrelevant_older_failures() {
        let history = vec![
            failure("x", "head"),
            failure("other", "other-head"),
            failure("x", "head"),
        ];
        let display_ids = BTreeMap::from([("x", "x")]);

        let repeated_failure = repeated_xpec_failure(&history, "head", &display_ids).unwrap();

        assert_eq!(repeated_failure.diff_from_oid, None);
    }

    #[test] // xpec: ex
    fn current_irrelevant_failure_prevents_a_repeated_xpec_warning() {
        let mut current = failure("x", "head");
        current.response_error = true;
        let history = vec![failure("x", "head"), current];
        let display_ids = BTreeMap::from([("x", "x")]);

        assert!(repeated_xpec_failure(&history, "head", &display_ids).is_none());
    }

    #[test] // xpec: 2Z,ex,gN,L
    fn current_resolution_does_not_conflate_a_prefix_reused_across_configs() {
        let root = temporary_git_project();
        let config = failure_history_config(&root);
        let mut state = XpecStateCache::default();

        let first = append_public_failure(&mut state, &root, &config, "same-full-id-one");
        let second = append_public_failure(&mut state, &root, &config, "same-full-id-two");
        let third = append_public_failure(&mut state, &root, &config, "same-full-id-two");

        assert!(!first.repeated_xpec_failure);
        assert!(!second.repeated_xpec_failure);
        assert!(third.repeated_xpec_failure);
        assert_eq!(second.short_id, "same");
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: L
    fn failure_history_persists_full_id_without_a_short_prefix_reference() {
        let json = serde_json::to_value(failure("full-expectation-id", "head")).unwrap();

        assert_eq!(
            json.get("xpecId").and_then(|value| value.as_str()),
            Some("full-expectation-id")
        );
        assert!(json.get("shortId").is_none());
        assert!(json.get("printedIdLength").is_none());
    }

    #[test] // xpec: ex
    fn repeated_diff_xpec_failure_carries_its_resolved_diff_from_oid() {
        let mut previous = failure("x", "head");
        previous.target_is_diff = true;
        previous.diff_from_oid = Some("previous-base".to_string());
        let mut current = failure("x", "head");
        current.target_is_diff = true;
        current.diff_from_oid = Some("current-base".to_string());
        let display_ids = BTreeMap::from([("x", "x")]);

        let repeated_failure =
            repeated_xpec_failure(&[previous, current], "head", &display_ids).unwrap();

        assert_eq!(
            repeated_failure.diff_from_oid.as_deref(),
            Some("current-base")
        );
    }

    #[test] // xpec: ex,fh,kL
    fn failure_history_rewrite_is_bounded_by_records_and_bytes() {
        assert!(!failure_history_needs_compaction(199, 100, 127));
        assert!(failure_history_needs_compaction(200, 100, 127));
        assert!(failure_history_needs_compaction(199, 100, 128));
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
