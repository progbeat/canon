use crate::check_errors::error_record_from_interrogation_error;
use crate::check_interrogation_state::{
    should_retry_full_scope_after_restricted_idk, CheckRuntime, InterrogationState,
};
use crate::check_model_fallback::interrogate_expectation_with_model_fallbacks;
use crate::check_narrowing::scope_narrowing_log_fields;
use crate::check_types::{CheckRecord, InterrogationResult, SelectedExpectation};
use crate::evaluator_types::EvaluatorRunner;
use crate::hash::full_scope;
use crate::history_reuse::is_reusable_history_record;
use crate::logging::DiagnosticLogWriter;
use crate::scope_hash::ScopeHashCache;
use std::path::Path;

pub(crate) struct InterrogationCall<'a> {
    pub(crate) root: &'a Path,
    pub(crate) runtime: &'a CheckRuntime<'a>,
    pub(crate) expectation: &'a SelectedExpectation,
    pub(crate) scope: &'a [String],
}

pub(crate) struct ScopedInterrogation<'a> {
    pub(crate) root: &'a Path,
    pub(crate) runtime: &'a CheckRuntime<'a>,
    pub(crate) expectation: &'a SelectedExpectation,
    pub(crate) enforced_scope: &'a mut Vec<String>,
}

impl<'a> ScopedInterrogation<'a> {
    fn call(&self) -> InterrogationCall<'_> {
        InterrogationCall {
            root: self.root,
            runtime: self.runtime,
            expectation: self.expectation,
            scope: self.enforced_scope,
        }
    }
}

pub(crate) fn interrogate_with_full_scope_retry<R: EvaluatorRunner>(
    call: ScopedInterrogation<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    interrogation_state: &mut InterrogationState,
    scope_hash_cache: &mut ScopeHashCache,
    break_after_tokens: Option<u64>,
) -> Result<InterrogationResult, String> {
    let mut interrogation = interrogate_or_error_record(
        call.call(),
        runner,
        diagnostic_log,
        interrogation_state,
        scope_hash_cache,
    )?;
    let should_stop_after_current_expectation =
        turn_exceeds_break_after_tokens(&interrogation, break_after_tokens)
            || turn_has_context_compaction(&interrogation);
    if should_retry_full_scope_after_restricted_idk(&interrogation.record, call.enforced_scope) {
        // `idk` is a non-answer, not a cache-spec "same answer" that can prove
        // a narrower scope. The interrogation policy requires a separate
        // full-scope retry, and that final record replaces the restricted
        // non-answer.
        *call.enforced_scope = full_scope();
        interrogation = interrogate_or_error_record(
            call.call(),
            runner,
            diagnostic_log,
            interrogation_state,
            scope_hash_cache,
        )?;
        interrogation.stop_after_current_expectation |= should_stop_after_current_expectation;
    } else if should_stop_after_current_expectation {
        return Ok(interrogation);
    }
    Ok(interrogation)
}

pub(crate) fn interrogate_or_error_record<R: EvaluatorRunner>(
    call: InterrogationCall<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    interrogation_state: &mut InterrogationState,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<InterrogationResult, String> {
    match interrogate_expectation_with_model_fallbacks(
        call.runtime,
        call.expectation,
        runner,
        diagnostic_log,
        interrogation_state,
        call.scope,
    ) {
        Ok(interrogation) => Ok(interrogation),
        Err(err) => Ok(InterrogationResult {
            record: error_record_from_interrogation_error(
                call.root,
                &call.runtime.config.agent,
                call.expectation,
                call.scope,
                &err,
                scope_hash_cache,
            )?,
            turn_usage: None,
            context_compacted: false,
            stop_after_current_expectation: false,
        }),
    }
}

pub(crate) fn turn_exceeds_break_after_tokens(
    interrogation: &InterrogationResult,
    break_after_tokens: Option<u64>,
) -> bool {
    let (Some(limit), Some(usage)) = (break_after_tokens, interrogation.turn_usage) else {
        return false;
    };
    usage.input_tokens.saturating_add(usage.output_tokens) > limit
}

pub(crate) fn turn_has_context_compaction(interrogation: &InterrogationResult) -> bool {
    interrogation.context_compacted
}

pub(crate) fn narrowed_scope_is_accepted(wide: &CheckRecord, narrowed: &CheckRecord) -> bool {
    // The evaluator proposes the smallest sufficient scope in its response, but
    // canon only trusts a strict narrowing after an independent interrogation
    // returns a reusable answer that is either unchanged or still incorrect.
    is_reusable_history_record(narrowed)
        && (narrowed.observed == wide.observed || !narrowed.passed())
}

pub(crate) fn restore_record_to_enforced_scope(
    mut record: CheckRecord,
    enforced_scope: &[String],
    enforced_scope_hash: String,
) -> CheckRecord {
    record.scope = enforced_scope.to_vec();
    record.scope_hash = enforced_scope_hash;
    record
}

pub(crate) fn write_scope_narrowing_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    id: &str,
    enforced_scope: &[String],
    record_scope: &[String],
    accepted: bool,
    initial_record: &CheckRecord,
    narrowed_record: &CheckRecord,
) -> Result<(), String> {
    let Some(writer) = diagnostic_log.as_deref_mut() else {
        return Ok(());
    };
    writer
        .write_event(
            "info",
            "scope.narrowing",
            &scope_narrowing_log_fields(
                id,
                enforced_scope,
                record_scope,
                accepted,
                initial_record,
                narrowed_record,
            ),
        )
        .map_err(|err| err.to_string())
}
