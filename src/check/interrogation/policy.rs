use crate::check::core::errors::error_record_from_interrogation_error;
use crate::check::core::types::{CheckRecord, InterrogationResult, SelectedExpectation};
use crate::check::interrogation::model_fallback::interrogate_expectation_with_model_fallbacks;
use crate::check::interrogation::narrowing::scope_narrowing_log_fields;
use crate::check::interrogation::state::{
    should_retry_full_scope_after_error, CheckRuntime, InterrogationRunState,
};
use crate::config_types::AgentConfig;
use crate::evaluator::EvaluatorRunner;
use crate::git::VisibleTreeOidCache;
use crate::hash::full_scope;
use crate::history::{is_reusable_history_record, HistoryCache};
use crate::logs::DiagnosticLogWriter;

pub(crate) struct InterrogationCall<'a> {
    pub(crate) runtime: &'a CheckRuntime<'a>,
    pub(crate) expectation: &'a SelectedExpectation,
    pub(crate) scope: &'a [String],
}

pub(crate) struct ScopedInterrogation<'a> {
    pub(crate) runtime: &'a CheckRuntime<'a>,
    pub(crate) expectation: &'a SelectedExpectation,
    pub(crate) enforced_scope: &'a mut Vec<String>,
}

impl<'a> ScopedInterrogation<'a> {
    fn call(&self) -> InterrogationCall<'_> {
        InterrogationCall {
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
    interrogation_run_state: &mut InterrogationRunState,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    break_after_tokens: Option<u64>,
) -> Result<InterrogationResult, String> {
    let mut interrogation = interrogate_or_error_record(
        call.call(),
        runner,
        diagnostic_log,
        interrogation_run_state,
        history_cache,
        visible_tree_oid_cache,
    )?;
    let should_stop_after_current_expectation =
        turn_exceeds_break_after_tokens(&interrogation, break_after_tokens)
            || turn_has_context_compaction(&interrogation);
    if should_retry_full_scope_after_error(
        interrogation.record.error.as_deref(),
        call.enforced_scope,
    ) {
        // Restricted insufficient-evidence is not final. Retry once with full
        // project scope and let that response become the record.
        *call.enforced_scope = full_scope();
        interrogation = interrogate_or_error_record(
            call.call(),
            runner,
            diagnostic_log,
            interrogation_run_state,
            history_cache,
            visible_tree_oid_cache,
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
    interrogation_run_state: &mut InterrogationRunState,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<InterrogationResult, String> {
    match interrogate_expectation_with_model_fallbacks(
        call.runtime,
        call.expectation,
        runner,
        diagnostic_log,
        interrogation_run_state,
        history_cache,
        call.scope,
    ) {
        Ok(interrogation) => Ok(interrogation),
        Err(err) => Ok(InterrogationResult {
            record: error_record_from_interrogation_error(
                call.runtime,
                &call.expectation.agent,
                call.expectation,
                call.scope,
                &err,
                visible_tree_oid_cache,
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

pub(crate) fn question_scope_suggestion_should_get_independent_verification(
    runtime: &CheckRuntime<'_>,
    agent: &AgentConfig,
    suggestion: Option<&[String]>,
    current_scope: &[String],
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<bool, String> {
    // Glossary-level q-scope suggestions are evaluator-provided claims. This
    // helper implements only the Interrogation Policy gate for whether such a
    // claim is worth an independent verification turn: at least 25% fewer
    // visible files. The response JSON Schema does not require repo-relative
    // or semantically sufficient paths; sufficiency is established only when
    // the independent verification produces an answer. A false result leaves
    // the evaluator's claim unverified; it does not redefine what a q-scope
    // suggestion is.
    let Some(suggestion) = suggestion else {
        return Ok(false);
    };
    let current_count = runtime.visible_file_count(visible_tree_oid_cache, agent, current_scope)?;
    if current_count == 0 {
        return Ok(false);
    }
    let suggested_count =
        match runtime.visible_file_count(visible_tree_oid_cache, agent, suggestion) {
            Ok(count) => count,
            Err(_) => return Ok(false),
        };
    Ok(suggested_count.saturating_mul(4) <= current_count.saturating_mul(3))
}

pub(crate) fn narrowed_scope_is_accepted(
    narrowed: &CheckRecord,
    proposed_scope: &[String],
) -> bool {
    // Acceptance means the q-scope suggestion graduated from evaluator claim
    // to verified reusable q-scope. Interrogation Policy requires the
    // independent verification turn to produce a schema-valid answer under
    // that proposed scope.
    is_reusable_history_record(narrowed)
        && verified_q_scope_answer_is_accepted(&narrowed.scope, proposed_scope)
}

pub(crate) fn verified_q_scope_answer_is_accepted(
    answer_scope: &[String],
    proposed_scope: &[String],
) -> bool {
    answer_scope == proposed_scope
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
