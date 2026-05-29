// Child test modules import this file with `super::*`; each module uses a
// different subset of this shared test prelude.
#![allow(unused_imports)]

use crate::app_server::{AppServerRunner, LazyAppServerRunner};
use crate::app_server_process::{configure_app_server_environment, prepare_evaluator_codex_home};
use crate::app_server_protocol::{
    app_server_error_message, app_server_error_value, app_server_failure_from_message,
    app_server_failure_from_value, app_server_message, append_completed_agent_text,
    context_compaction_event, token_usage_update, turn_idle_timed_out, turn_started_id, turn_text,
};
use crate::app_server_runner::{normalize_app_server_evaluator_response, turn_start_request};
use crate::app_server_usage::{
    carryover_tokens, record_context_compaction_event, record_token_usage_update,
    thread_reuse_policy_should_retire,
};
use crate::check::run_check_with_runner;
use crate::check_command::{
    check_command_writes_agent_message, prepare_check_execution, run_check_command,
};
use crate::check_command_args::{check_help_requested, parse_check_command_args};
use crate::check_command_finish::{
    check_agent_message, check_agent_messages, pass_improvement_notice, staged_pass_notice_count,
    staged_passes_failed_at_head_count,
};
use crate::check_config::{parse_check_config_content, parse_check_config_content_with_root};
use crate::check_errors::error_record_from_interrogation_error;
use crate::check_generator_paths::{expand_filesystem_generator_paths, expand_generator_paths};
use crate::check_interrogation::{
    ask_with_reused_thread, interrogate_expectation_with_model, ThreadTurnRequest,
};
use crate::check_interrogation_records::finalize_interrogation_response;
use crate::check_interrogation_state::{
    evaluator_thread_reuse_key, should_retry_full_scope_after_restricted_response, CheckRuntime,
    InterrogationRunState,
};
use crate::check_lazy_reset::{
    apply_scheduled_lazy_full_scope_resets, lazy_full_scope_reset_count,
    plan_lazy_full_scope_reset, schedule_lazy_full_scope_resets,
    set_non_selected_expectation_scopes_to_full,
};
use crate::check_model_fallback::{
    interrogate_expectation_with_model_fallbacks, run_with_model_fallbacks,
    write_model_fallback_events,
};
use crate::check_narrowing::scope_narrowing_log_fields;
use crate::check_order_state::{latest_recorded_non_pass_timestamp, write_latest_non_pass_record};
use crate::check_output::{
    escape_check_output_text, pad_summary_line, record_requires_human_review,
    render_check_output_record, render_check_summary, render_query_output,
    render_token_usage_summary, report_output_skipped_count, write_and_flush_result_output,
    write_query_output, write_summary_line,
};
use crate::check_preflight::{
    is_canon_only_staged_change_bytes, is_canon_project_path_bytes, staged_changed_path_bytes,
    staged_changed_paths, staged_changed_paths_from_name_status_z,
};
use crate::check_query::run_query_with_runner;
use crate::check_query_command::run_check_query_command;
use crate::check_reporting::{
    collect_check_token_usage, print_token_usage_summary, write_check_finish_event,
};
use crate::check_selection::{
    expectation_identities, initial_non_selected_expectations,
    order_expectations_by_latest_non_pass, parse_check_options, parse_cooldown,
    select_expectations,
};
use crate::check_types::{
    check_run_error, CachedExpectation, CheckCommandArgs, CheckOptions, CheckRecord, CheckResult,
    CheckRunError, CheckRunReport, Cooldown, EvaluatorResponseJson, InterrogationResult,
    NarrowingStats, ObservedAnswerState, ParsedAnswer, QueryResult, SelectedExpectation,
};
use crate::check_validation::{
    check_config_loads_plugins, codex_reasoning_effort, normalize_agent_ignore_pattern_for_config,
    validate_check_config, validate_optional_model, validate_plugin_config_key,
    validate_relative_config_path,
};
use crate::cli::{command_error_has_public_diagnostic, run, CommandError};
use crate::config_types::{
    AgentConfig, CheckConfig, Expectation, RawCheckConfig, RawExpectationItem,
};
use crate::evaluator::{
    evaluator_response_output_schema, evaluator_turn_input, render_evaluator_turn_input,
};
use crate::evaluator_config::{
    app_server_args, app_server_model_key, app_server_startup_config_args_with_no_sandbox,
    app_server_startup_filesystem_arg, evaluator_model_catalog_json, evaluator_thread_config,
    evaluator_thread_config_with_no_sandbox, evaluator_working_tree_permissions,
    thread_reuse_carryover_token_target_arg, toml_string,
};
use crate::evaluator_prompt::{developer_instructions, EVALUATOR_BASE_INSTRUCTIONS};
use crate::evaluator_response::parse_evaluator_response;
use crate::evaluator_response_cache::{response_excerpt, EvaluatorResponseParseCache};
use crate::evaluator_scope::{parse_scope_json, parse_scope_strings};
use crate::evaluator_turn::{
    ask_and_log, ask_once, effective_thinking, evaluator_models, is_context_window_failure,
    is_model_technical_failure, model_label, record_from_response,
    session_failure_invalidates_thread, EvaluatorFailureKind, EvaluatorTurnContext,
};
use crate::evaluator_types::{EvaluatorError, EvaluatorRunner};
use crate::fs_util::{ensure_dir, for_each_nonempty_line, replace_file_with_temp};
use crate::gate::*;
use crate::git::resolve_git_path;
#[cfg(unix)]
use crate::git::{read_git_blobs_with_git_program, GitBlobReader};
use crate::git_config::{git_config_get, GitConfigGetError};
use crate::hash::{expectation_id, fnv64_with_seed, full_scope, hash_120, hash_key};
use crate::history::{
    history_file_name, history_path, parse_history_record_line, read_history_records,
    read_history_records_from_path, HistoryCache,
};
use crate::history_append::{
    append_current_history_record_with_cache, append_history_record,
    append_history_record_with_cache,
};
use crate::history_cache_key::history_cache_key;
use crate::history_cleanup::{active_expectation_ids, cleanup_stale_cache_dirs};
use crate::history_compaction::{
    compact_history, compact_history_temp_path, compact_repository_history, should_compact_history,
    should_compact_history_for_seed,
};
use crate::history_reuse::{
    cooldown_history_record, is_reusable_history_record,
    latest_history_record_matching_visible_tree_oid, latest_stored_q_scope_with_cache,
    same_tree_history_record, same_tree_history_record_with_cache,
};
use crate::hooks::*;
use crate::logging::{
    append_runtime_log_event, diagnostic_log_config, push_json_control_escape,
    render_answer_history_record, render_runtime_log_event, stale_diagnostic_log_lock_age,
    write_diagnostic_log, write_diagnostic_log_lock_token, DiagnosticLogWriter,
};
use crate::logging_config::{
    parse_carryover_token_target, thread_reuse_config, DEFAULT_THREAD_REUSE_CONFIG,
};
use crate::notes::*;
use crate::notes_cli::{arg_to_string, collect_text, INDEX_LOCK_STALE_AFTER_SECS};
use crate::notes_header::{
    header, initial_content, normalize_body, parse_key_from_header, validate_note_key,
    verify_note_key, verify_note_key_from_first_line,
};
use crate::notes_index::{
    lock_index, read_index, remove_index, stale_index_lock_age, upsert_index, validate_index_entry,
    write_file_atomically, INDEX_COMPACT_MIN_BYTES,
};
use crate::notes_restore::{
    error_with_restore_context, restore_deleted_note_after_index_failure,
    restore_note_after_index_failure,
};
use crate::output::{
    write_stderr_bytes, write_stderr_line, write_stdout, write_stdout_bytes, write_stdout_line,
};
#[cfg(unix)]
use crate::platform::git_path_from_raw_bytes;
use crate::platform::path_from_git_stdout;
use crate::project::{command_output_trimmed, git_project_root};
use crate::project_types::{Config, Note};
use crate::repo_inspection::RepoInspectionCache;
use crate::scope::{
    effective_ignore_patterns, is_denied_path, is_denied_path_bytes, is_strict_scope_subset,
    normalize_repo_path, path_bytes_in_scope, sanitize_scope, sanitize_scope_for_hash,
    scope_contains, scope_is_within,
};
use crate::staged_worktree::snapshot_parent_outside_worktree;
use crate::staged_worktree::StagedWorktreeView;
use crate::time::{format_record_timestamp, parse_record_timestamp, unix_timestamp};
use crate::token_usage_types::{
    reference_token_cost, ContextCompactionEvent, EvaluatorTurnUsage, TokenUsage, TokenUsageUpdate,
};
use crate::tree_source::TreeSource;
#[cfg(unix)]
use crate::visible_tree_oid::staged_scope_entries;
use crate::visible_tree_oid::{
    gate_head_tree_fingerprint, normalize_index_metadata, sha1_visible_tree_oid_from_entries,
    staged_visible_tree_oid, VisibleTreeOidCache,
};
use crate::{
    APP_SERVER_TURN_TIMEOUT_SECS, CANON_CACHE_DIR_GIT_PATH, CANON_LOG_DIR_GIT_PATH, CHECK_PATH,
    DEFAULT_CHECK_CONFIG_SOURCE, DEFAULT_PRE_COMMIT_HOOK, ERROR_INSUFFICIENT_EVIDENCE,
    ERROR_INVALID_QUESTION, ERROR_UNPARSABLE, GIT_HOOKS_PATH, HISTORY_COMPACT_CHANCE_DENOMINATOR,
    HISTORY_COMPACT_KEEP_RECORDS, PRE_COMMIT_HOOK_PATH, RESULT_FAIL, RESULT_PASS,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

mod test_check_support;
mod test_env;
mod test_git_support;

// Test files import `super::*` as a compact test prelude. Keep the helper
// surface explicit here so ownership still points back to the fixture module
// that defines each helper.
pub(crate) use test_check_support::{
    answer, check_config_yaml, check_options, error_response, expectation_record,
    parse_check_config, sample_record, test_selector, FakeRunner, FlushCountingWriter,
};
pub(crate) use test_env::{temp_home, test_path, with_env, EnvSnapshot, TestDir, ENV_LOCK};
pub(crate) use test_git_support::{commit_all, git_project, write_check_config};

pub(crate) fn enable_diagnostic_logs(root: &Path) {
    let output = Command::new("git")
        .args(["config", "canon.logs.maxSize", "1M"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(crate) fn stale_visible_tree_oid() -> String {
    "0000000000000000000000000000000000000000".to_string()
}

pub(crate) fn append_legacy_history_record(
    root: &Path,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
) {
    let path = history_path(root, expectation).unwrap();
    ensure_dir(path.parent().unwrap()).unwrap();
    let mut value = serde_json::Map::new();
    value.insert("timestamp".to_string(), json!(record.timestamp));
    value.insert("observed".to_string(), json!(record.observed));
    if let Some(error) = &record.error {
        value.insert("error".to_string(), json!(error));
    }
    value.insert("evidence".to_string(), json!(record.evidence));
    value.insert("qScope".to_string(), json!(record.scope));
    value.insert("visibleTreeOid".to_string(), json!(record.visible_tree_oid));
    value.insert("result".to_string(), json!(record.result));
    value.insert("id".to_string(), json!(record.id));
    if let Some(prompt) = &record.prompt {
        value.insert("prompt".to_string(), json!(prompt));
    }
    if let Some(expected) = &record.expected {
        value.insert("expected".to_string(), json!(expected));
    }
    if let Some(cache_key) = &record.cache_key {
        value.insert("cacheKey".to_string(), json!(cache_key));
    }
    let line = serde_json::to_string(&Value::Object(value)).unwrap();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    file.write_all(line.as_bytes()).unwrap();
    file.write_all(b"\n").unwrap();
    file.flush().unwrap();
}

mod tests_app_server_process;
mod tests_app_server_protocol;
mod tests_check_command_args;
mod tests_check_config_validation;
mod tests_check_core;
mod tests_check_lazy_reset;
mod tests_check_model_recovery;
mod tests_check_parser;
mod tests_check_query_output;
mod tests_check_response_review;
mod tests_check_restricted_scope;
mod tests_cli_aliases;
mod tests_config_env;
mod tests_evaluator_permissions;
mod tests_evaluator_prompt;
mod tests_gate;
mod tests_generator_config;
mod tests_git_runtime;
mod tests_hash;
mod tests_history_cached_check;
mod tests_history_cooldown;
mod tests_history_files;
mod tests_history_same_tree_reuse;
mod tests_hook_install;
mod tests_init;
mod tests_logging_runtime;
mod tests_notes_crud;
mod tests_notes_index_behavior;
mod tests_scope_runtime;
mod tests_staged_preflight;
mod tests_time;
