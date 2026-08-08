use super::super::{ThreadTurnContext, ThreadTurnRequest};
use crate::check::evaluator_response_output_schema_for_scope;
use crate::check::expectation_inspection::{CanonShowContext, CanonShowDynamicToolHandler};
use crate::evaluator::{
    ask_once as ask_evaluator_once, write_thread_lifecycle_event, EvaluatorAttemptRequest,
    EvaluatorDynamicToolHandler, EvaluatorError, EvaluatorRunner, EvaluatorTurnContext,
    ParsedTurnResponse, ThreadLifecycleLog,
};
use std::collections::BTreeSet;

pub(super) fn log_thread_lifecycle_and_ask<R: EvaluatorRunner>(
    context: &mut ThreadTurnContext<'_, '_, '_, R>,
    lifecycle_log: &ThreadLifecycleLog,
    request: &ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    // Runtime logs expose the effective instructions for every thread start or
    // reuse, so thread behavior can be audited from the log without reading
    // derived state.
    write_thread_lifecycle_event(
        context.diagnostic_log,
        lifecycle_log,
        request.expectation_id,
        request.enforced_scope,
        request.model,
        request.thinking,
    )?;
    ask_current_thread(context, &lifecycle_log.thread_id, request)
}

fn ask_current_thread<R: EvaluatorRunner>(
    context: &mut ThreadTurnContext<'_, '_, '_, R>,
    thread_id: &str,
    request: &ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let turn = EvaluatorTurnContext {
        thread_id,
        model: request.model,
        thinking: request.thinking,
    };
    let schema_scope = request
        .response_contract
        .schema_scope(context.runtime, request.enforced_scope);
    let output_schema = evaluator_response_output_schema_for_scope(
        schema_scope,
        request.short_id,
        request.diff_target_prior_answer,
    );
    let answered_short_ids = context
        .interrogation_session
        .thread_state()
        .thread_registry()
        .thread_answered_short_ids(thread_id);
    let (response, shown_expectation_ids) = {
        let response_parse_memo = context
            .interrogation_session
            .thread_state_mut()
            .response_parse_memo_mut();
        let mut ask = |dynamic_tool_handler: Option<&mut dyn EvaluatorDynamicToolHandler>| {
            ask_evaluator_once(
                context.runner,
                response_parse_memo,
                context.diagnostic_log,
                EvaluatorAttemptRequest {
                    attempt: request.attempt,
                    turn: &turn,
                    task_input: request.task_input,
                    schema_scope,
                    output_schema: &output_schema,
                    short_id: request.short_id,
                    answered_short_ids: &answered_short_ids,
                    expectation_id: request.expectation_id,
                },
                dynamic_tool_handler,
            )
        };
        if request.canon_show_dynamic_tools_enabled() {
            let mut dynamic_tool_handler = CanonShowDynamicToolHandler::new(
                CanonShowContext {
                    root: context.runtime.root,
                    config: context.runtime.config,
                    identities: context.runtime.expectation_identities,
                    tree_source: context.runtime.tree_source(),
                    current_expectation_id: request.expectation_id,
                },
                context.xpec_state,
                context.visible_tree_oid_cache,
            );
            let response = ask(Some(&mut dynamic_tool_handler));
            let shown_expectation_ids = dynamic_tool_handler.into_shown_expectation_ids();
            (response, shown_expectation_ids)
        } else {
            (ask(None), BTreeSet::new())
        }
    };
    // xpec: F
    // The dynamic tool handler records the expectation IDs actually rendered
    // into `canon.show` output; future reuse lookups reject this thread for
    // those expectation IDs.
    context
        .interrogation_session
        .thread_state_mut()
        .thread_registry_mut()
        .record_thread_canon_show_expectation_ids(thread_id, shown_expectation_ids);
    let response = response?;
    if response.schema_valid {
        context
            .interrogation_session
            .thread_state_mut()
            .thread_registry_mut()
            .record_thread_answered_short_id(thread_id, request.short_id);
    }
    Ok(response)
}
