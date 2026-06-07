use crate::check::{CheckRecord, CheckResult, EvaluatorResponseJson};
use crate::config_types::AgentConfig;
use crate::evidence::evidence_file_refs_are_visible;
use crate::fs_util::for_each_nonempty_line;
use crate::git::{git_object_oid_has_hex_len, git_object_oid_has_known_shape, VisibleTreeOidCache};
use crate::logs::{external_log_error, DiagnosticLogError, DiagnosticLogResult};
use crate::scope::visible_scope;
use crate::time::parse_record_timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

// Cache-spec answer history records are JSON Lines with the required field
// prefix rendered here. Runtime history reads derive result from the current
// expectation instead of trusting persisted result metadata.

pub(super) fn history_file_name() -> &'static str {
    "history.jsonl"
}

pub(super) fn read_repository_history_records_from_path(
    root: &Path,
    path: &Path,
    expected_answer: &str,
) -> Result<Vec<CheckRecord>, String> {
    let native_oid_hex_len =
        VisibleTreeOidCache::new().repository_native_object_oid_hex_len(root)?;
    let mut records = Vec::new();
    for_each_nonempty_line(path, |line_number, line| {
        match parse_history_record_line_for_expected(
            path,
            line_number,
            &line,
            Some(expected_answer),
        ) {
            Ok(record) => {
                if git_object_oid_has_hex_len(&record.visible_tree_oid, native_oid_hex_len)
                    && evidence_file_refs_are_visible(&record.evidence, &record.scope)
                {
                    records.push(record);
                } else {
                    // A history row is reusable only when its persisted tree
                    // hash has the repository-native shape and its file
                    // evidence is supported by the persisted visible scope.
                    // Rows that fail either cache-integrity check are ignored
                    // like other corrupt cache data.
                }
            }
            Err(_) => {
                // History is a reusable cache, not authoritative project data.
                // Corrupt cache lines are ignored here and dropped by the same
                // parser during compaction, while real file I/O errors still
                // propagate from `for_each_nonempty_line`.
            }
        }
        Ok(())
    })?;
    Ok(records)
}

pub(super) fn parse_history_record_line(
    path: &Path,
    line_number: usize,
    line: &str,
) -> Result<CheckRecord, String> {
    // Shape-only cache maintenance parse. Without a current expectation there
    // is no current expected answer to compare against, so schema-valid answer
    // rows parse as neutral passes. Runtime history reads must use
    // `read_repository_history_records_from_path`, which derives `result` from
    // the current expectation's expected answer.
    parse_history_record_line_for_expected(path, line_number, line, None)
}

fn parse_history_record_line_for_expected(
    path: &Path,
    line_number: usize,
    line: &str,
    expected_answer: Option<&str>,
) -> Result<CheckRecord, String> {
    let record = serde_json::from_str::<HistoryReadRecord>(line).map_err(|err| {
        format!(
            "invalid history JSON in {} line {}: {}",
            path.display(),
            line_number,
            err
        )
    })?;
    let record = record.into_check_record(expected_answer);
    validate_schema_valid_answer_history_record(&record).map_err(|message| {
        format!(
            "invalid answer history record in {} line {}: records must be schema-valid responses with answer: {}",
            path.display(),
            line_number,
            message
        )
    })?;
    Ok(record)
}

#[derive(Deserialize)]
struct HistoryReadRecord {
    timestamp: String,
    observed: String,
    evidence: String,
    #[serde(rename = "visibleScope", alias = "qScope", alias = "scope")]
    visible_scope: Vec<String>,
    #[serde(rename = "visibleTreeOid")]
    visible_tree_oid: String,
    #[serde(default)]
    #[serde(flatten)]
    extra_fields: BTreeMap<String, Value>,
}

impl HistoryReadRecord {
    fn into_check_record(self, expected_answer: Option<&str>) -> CheckRecord {
        let has_error_field = self.extra_fields.contains_key("error");
        let result = expected_answer
            .map(|expected| CheckResult::from_expected_answer(expected, &self.observed))
            .unwrap_or(CheckResult::Pass);
        CheckRecord {
            timestamp: self.timestamp,
            number: 0,
            result,
            prompt: None,
            expected: expected_answer.map(str::to_string),
            observed: self.observed,
            error: has_error_field.then(|| "error".to_string()),
            evidence: self.evidence,
            scope: self.visible_scope,
            suggested_q_scope: None,
            visible_tree_oid: self.visible_tree_oid,
            id: String::new(),
            display_id: String::new(),
        }
    }
}

fn validate_schema_valid_answer_history_record(record: &CheckRecord) -> Result<(), String> {
    // `HistoryReadRecord` requires the Cache spec prefix fields with no
    // defaults, while allowing extra metadata because the spec says "at least"
    // those fields. Cache records store the evaluator response's `answer` value
    // as `observed`, so reconstruct a minimal answer response and validate it
    // with the same evaluator response schema used at runtime.
    if record.error.is_some() {
        return Err("error responses are not answer history records".to_string());
    }
    validate_history_answer_response_schema(record)?;
    if parse_record_timestamp(&record.timestamp).is_none() {
        return Err("timestamp must be UTC in YYYY-MM-DDTHH:MM:SSZ form".to_string());
    }
    if has_duplicate_scope_entries(&record.scope) {
        return Err("visibleScope must not contain duplicate entries".to_string());
    }
    if !git_object_oid_has_known_shape(&record.visible_tree_oid) {
        return Err("visibleTreeOid must be a Git object ID hex string".to_string());
    }
    Ok(())
}

fn has_duplicate_scope_entries(scope: &[String]) -> bool {
    let mut seen = Vec::new();
    for entry in scope {
        if seen.iter().any(|existing| *existing == entry) {
            return true;
        }
        seen.push(entry);
    }
    false
}

fn validate_history_answer_response_schema(record: &CheckRecord) -> Result<(), String> {
    let response = EvaluatorResponseJson {
        answer: Some(record.observed.clone()),
        error: None,
        evidence: record.evidence.clone(),
        // History rows store the cache-required answer fields, not the full
        // evaluator response. Use a schema-valid placeholder so this check
        // continues to validate the persisted answer/evidence contract.
        q_scope_suggestion: vec![".".to_string()],
    };
    response.validate_schema().map_err(|message| {
        format!("observed must match evaluator response answer schema: {message}")
    })
}

pub(super) fn validate_appendable_answer_history_record(
    record: &CheckRecord,
    native_oid_hex_len: usize,
) -> Result<(), String> {
    // Append-time validation checks that a runtime-produced record is a valid
    // answer-history row and that its `visibleTreeOid` uses the repository's
    // native object format. It intentionally does not recompute the stored
    // visible scope's current visible tree here: history rows are later read
    // when their stored visibleScope may describe an older Git state, and cache
    // reuse is the layer that compares stored OIDs with freshly computed
    // current OIDs.
    validate_schema_valid_answer_history_record(record)?;
    if !git_object_oid_has_hex_len(&record.visible_tree_oid, native_oid_hex_len) {
        return Err(
            "visibleTreeOid must match this repository's Git object hash algorithm".to_string(),
        );
    }
    Ok(())
}

pub(super) fn render_answer_history_record(
    agent: &AgentConfig,
    record: &CheckRecord,
) -> DiagnosticLogResult<String> {
    validate_schema_valid_answer_history_record(record)
        .map_err(|message| external_log_error("render answer history record", message))?;
    // Keep answer-history rows to the Cache spec fields. Current result is
    // derived from observed vs the current expectation rather than persisted.
    let history = HistoryLogRecord {
        timestamp: &record.timestamp,
        observed: &record.observed,
        evidence: &record.evidence,
        visible_scope: &visible_scope(agent, &record.scope)
            .map_err(|message| external_log_error("render answer history record", message))?,
        visible_tree_oid: &record.visible_tree_oid,
    };
    answer_history_json_line(&history)
}

fn answer_history_json_line(value: &impl Serialize) -> DiagnosticLogResult<String> {
    let mut output = serde_json::to_string(value).map_err(|source| DiagnosticLogError::Json {
        description: "history log record",
        source,
    })?;
    output.push('\n');
    Ok(output)
}

#[derive(Serialize)]
struct HistoryLogRecord<'a> {
    // Required Cache spec prefix. Keep these fields first and in this order:
    // timestamp, observed, evidence, visibleScope, visibleTreeOid.
    timestamp: &'a str,
    observed: &'a str,
    evidence: &'a str,
    #[serde(rename = "visibleScope")]
    visible_scope: &'a [String],
    #[serde(rename = "visibleTreeOid")]
    visible_tree_oid: &'a str,
}

#[cfg(test)]
mod tests {
    use super::{parse_history_record_line, render_answer_history_record};
    use crate::check::{CheckRecord, CheckResult};
    use crate::config_types::AgentConfig;
    use serde_json::Value;
    use std::path::Path;

    #[test]
    fn answer_history_record_writes_visible_scope_field() {
        let agent = AgentConfig {
            models: Vec::new(),
            thinking: "medium".to_string(),
            ignore: vec!["target/**".to_string()],
            plugins: Vec::new(),
        };
        let record = CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: 1,
            result: CheckResult::Pass,
            prompt: Some("Does it pass?".to_string()),
            expected: Some("yes".to_string()),
            observed: "yes".to_string(),
            error: None,
            evidence: "`src/main.rs`".to_string(),
            scope: vec![".".to_string()],
            suggested_q_scope: None,
            visible_tree_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            id: "11111111111111111111".to_string(),
            display_id: "1".to_string(),
        };

        let line = render_answer_history_record(&agent, &record).unwrap();
        let json: Value = serde_json::from_str(&line).unwrap();

        assert!(json.get("qScope").is_none());
        assert_eq!(
            json.get("visibleScope"),
            Some(&serde_json::json!([".", ":(exclude,glob)target/**"]))
        );
    }

    #[test]
    fn answer_history_rejects_duplicate_visible_scope_entries() {
        let line = r#"{"timestamp":"1970-01-01T00:00:00Z","observed":"yes","evidence":"ok","visibleScope":["src","src"],"visibleTreeOid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#;

        let err = parse_history_record_line(Path::new("history.jsonl"), 1, line).unwrap_err();

        assert!(err.contains("visibleScope must not contain duplicate entries"));
    }
}
