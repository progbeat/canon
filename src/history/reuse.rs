// Answer-history lookup for the Cache spec: newest-to-oldest history scanning
// plus current visibleTreeOid matching.
use crate::check::{CheckRecord, CheckResult, SelectedExpectation};
use crate::config_types::AgentConfig;
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::history::HistoryCache;
use crate::scope::sanitize_scope_for_hash;
use crate::time::parse_record_timestamp;
use std::path::Path;

pub(crate) fn same_tree_history_record_with_cache(
    root: &Path,
    source: &TreeSource,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Option<CheckRecord>, String> {
    latest_history_record_matching_visible_tree_oid(root, expectation, history_cache, |scope| {
        visible_tree_oid_cache.visible_tree_oid_for_reuse(root, source, agent, scope)
    })
}

pub(crate) fn latest_history_record_matching_visible_tree_oid(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    mut current_visible_tree_oid_for_scope: impl FnMut(&[String]) -> Result<Option<String>, String>,
) -> Result<Option<CheckRecord>, String> {
    // Cache lookup follows the Cache spec's answer-history contract: only
    // schema-valid answer records loaded by the history reader can reach this
    // newest-to-oldest visibleTreeOid match.
    let matched_record =
        scan_latest_history_records(root, expectation, history_cache, |mut record| {
            let Ok(scope) = sanitize_scope_for_hash(&record.scope) else {
                return Ok(HistoryRecordScan::Continue);
            };
            let Some(current_visible_tree_oid) = current_visible_tree_oid_for_scope(&scope)? else {
                return Ok(HistoryRecordScan::Continue);
            };
            if current_visible_tree_oid == record.visible_tree_oid {
                record.scope = scope;
                return Ok(HistoryRecordScan::Done(Some(record)));
            }
            Ok(HistoryRecordScan::Continue)
        })?;
    Ok(matched_record.map(|record| record_with_current_expectation(record, expectation)))
}

pub(crate) fn cooldown_history_record(
    root: &Path,
    _agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    now: u64,
) -> Result<Option<CheckRecord>, String> {
    let Some(cooldown) = expectation.cooldown else {
        return Ok(None);
    };
    let record = scan_latest_history_records(root, expectation, history_cache, |mut record| {
        // Cooldown keys off the latest answer history record, unlike same-tree
        // lookup which searches for the latest visibleTreeOid match. A newer
        // invalid scope, fail, bad timestamp, or expired result deliberately
        // blocks cooldown reuse of an older result.
        let Ok(scope) = sanitize_scope_for_hash(&record.scope) else {
            return Ok(HistoryRecordScan::Done(None));
        };
        record.scope = scope;
        let Some(timestamp) = parse_record_timestamp(&record.timestamp) else {
            return Ok(HistoryRecordScan::Done(None));
        };
        let result = current_result_for_history_record(&record, expectation);
        let Some(duration) = cooldown.duration_for(result) else {
            return Ok(HistoryRecordScan::Done(None));
        };
        if now.saturating_sub(timestamp) >= duration {
            return Ok(HistoryRecordScan::Done(None));
        }
        // Cooldown is not a same-tree lookup: a fresh latest configured result
        // can be reused even when its visibleTreeOid differs from the current
        // evaluator-visible tree.
        Ok(HistoryRecordScan::Done(Some(record)))
    })?;
    Ok(record.map(|record| cooldown_record_with_current_expectation(record, expectation)))
}

pub(crate) enum CachedHistoryRecord {
    SameTree(CheckRecord),
    Cooldown(CheckRecord),
}

pub(crate) fn cached_history_record(
    same_tree: Option<CheckRecord>,
    cooldown: Option<CheckRecord>,
) -> Option<CachedHistoryRecord> {
    // Cached Result gives same-tree records priority; cooldown is only a
    // fallback when no same-tree record exists.
    match (same_tree, cooldown) {
        (Some(record), _) => Some(CachedHistoryRecord::SameTree(record)),
        (None, Some(record)) => Some(CachedHistoryRecord::Cooldown(record)),
        (None, None) => None,
    }
}

pub(crate) fn latest_stored_q_scope_with_cache(
    root: &Path,
    _agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<Option<Vec<String>>, String> {
    // Expectation-mode `canon check` calls this before each fresh interrogation.
    // It returns only the latest stored q-scope from answer history; it is not a
    // cached check result and does not let callers skip evaluator work. Cache
    // specifies that answer-history records contain schema-valid `answer`
    // responses only, and each record's `qScope` is the q-scope used to form
    // that record's visible tree.
    scan_latest_history_records(root, expectation, history_cache, |record| {
        let Some(scope) = sanitized_answer_history_q_scope(&record) else {
            return Ok(HistoryRecordScan::Continue);
        };
        Ok(HistoryRecordScan::Done(Some(scope)))
    })
}

enum HistoryRecordScan<T> {
    Continue,
    Done(Option<T>),
}

fn scan_latest_history_records<T>(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    mut scan: impl FnMut(CheckRecord) -> Result<HistoryRecordScan<T>, String>,
) -> Result<Option<T>, String> {
    let records = history_cache.read_records(root, expectation)?;
    for record in records.into_iter().rev() {
        match scan(record)? {
            HistoryRecordScan::Continue => {}
            HistoryRecordScan::Done(value) => return Ok(value),
        }
    }
    Ok(None)
}

fn sanitized_answer_history_q_scope(record: &CheckRecord) -> Option<Vec<String>> {
    sanitize_scope_for_hash(&record.scope).ok()
}

fn record_with_current_expectation(
    mut record: CheckRecord,
    expectation: &SelectedExpectation,
) -> CheckRecord {
    // The reusable lookup cache stores the raw matching history record. Current
    // display metadata is applied after lookup so moving or editing an
    // expectation during the same operation cannot make the cached value stale.
    record.id = expectation.id.clone();
    record.display_id = expectation.display_id.clone();
    record.number = expectation.number;
    record.prompt = Some(expectation.q.clone());
    record.expected = Some(expectation.a.clone());
    record.result = current_result_for_history_record(&record, expectation);
    record
}

fn cooldown_record_with_current_expectation(
    record: CheckRecord,
    expectation: &SelectedExpectation,
) -> CheckRecord {
    let mut record = record_with_current_expectation(record, expectation);
    record.result = CheckResult::Pass;
    record.observed = expectation.a.clone();
    record.error = None;
    record
}

pub(crate) fn is_reusable_history_record(record: &CheckRecord) -> bool {
    // Runtime persistence uses "reusable" to mean "schema-valid answer
    // response". Fail answers are reusable cache records; evaluator errors and
    // unparsable review records are not answer history.
    record.expected_text().is_some() && record.error.is_none()
}

fn current_result_for_history_record(
    record: &CheckRecord,
    expectation: &SelectedExpectation,
) -> CheckResult {
    CheckResult::from_expected_answer(&expectation.a, &record.observed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::Cooldown;
    use crate::time::format_record_timestamp;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn cached_history_record_prefers_same_tree_over_newer_cooldown() {
        let same_tree = record("1970-01-01T00:00:00Z", "same-tree");
        let cooldown = record("2099-01-01T00:00:00Z", "cooldown");

        let hit = cached_history_record(Some(same_tree), Some(cooldown)).unwrap();

        match hit {
            CachedHistoryRecord::SameTree(record) => assert_eq!(record.evidence, "same-tree"),
            CachedHistoryRecord::Cooldown(_) => panic!("cooldown must be only a fallback"),
        }
    }

    fn record(timestamp: &str, evidence: &str) -> CheckRecord {
        CheckRecord {
            timestamp: timestamp.to_string(),
            number: 1,
            result: CheckResult::Pass,
            prompt: Some("Does it pass?".to_string()),
            expected: Some("yes".to_string()),
            observed: "yes".to_string(),
            error: None,
            evidence: evidence.to_string(),
            scope: vec![".".to_string()],
            suggested_q_scope: None,
            visible_tree_oid: "tree".to_string(),
            id: "11111111111111111111".to_string(),
            display_id: "1".to_string(),
            cache_key: None,
        }
    }

    #[test]
    fn cooldown_history_record_invalid_latest_scope_blocks_older_reuse() {
        let root = git_project("cooldown-invalid-latest-scope");
        let expectation = expectation_with_cooldown();
        let mut history_cache = HistoryCache::default();
        let path = history_cache.path(&root, &expectation).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                history_line(10, r#"["."]"#, "older pass"),
                history_line(20, r#"[".."]"#, "newer invalid scope")
            ),
        )
        .unwrap();

        let hit = cooldown_history_record(
            &root,
            &expectation.agent,
            &expectation,
            &mut history_cache,
            30,
        )
        .unwrap();

        assert!(hit.is_none(), "invalid latest q-scope must block cooldown");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn same_tree_history_record_skips_absent_scope() {
        let root = git_project("same-tree-absent-scope");
        let expectation = expectation_with_cooldown();
        let mut history_cache = HistoryCache::default();
        let path = history_cache.path(&root, &expectation).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                history_line(10, r#"["."]"#, "older pass"),
                history_line(20, r#"["missing.rs"]"#, "newer absent scope")
            ),
        )
        .unwrap();

        let hit = latest_history_record_matching_visible_tree_oid(
            &root,
            &expectation,
            &mut history_cache,
            |scope| {
                if scope == ["missing.rs"] {
                    Ok(None)
                } else {
                    Ok(Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()))
                }
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(hit.evidence, "older pass");

        let _ = fs::remove_dir_all(root);
    }

    fn expectation_with_cooldown() -> SelectedExpectation {
        SelectedExpectation {
            number: 1,
            id: "11111111111111111111".to_string(),
            display_id: "1".to_string(),
            q: "Does it pass?".to_string(),
            a: "yes".to_string(),
            agent: AgentConfig {
                models: Vec::new(),
                thinking: "medium".to_string(),
                ignore: Vec::new(),
                plugins: Vec::new(),
            },
            cooldown: Some(Cooldown {
                pass_seconds: Some(100),
                fail_seconds: None,
            }),
        }
    }

    fn history_line(timestamp: u64, q_scope: &str, evidence: &str) -> String {
        format!(
            r#"{{"timestamp":"{}","observed":"yes","evidence":"{}","qScope":{},"visibleTreeOid":"{}"}}"#,
            format_record_timestamp(timestamp),
            evidence,
            q_scope,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        )
    }

    fn git_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("canon-test-{}-{}-{}", name, process::id(), unique));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let output = Command::new("git")
            .arg("init")
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        root
    }
}
