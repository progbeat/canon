use crate::check_interrogation::interrogate_expectation_with_model;
use crate::check_interrogation_state::{CheckRuntime, InterrogationState};
use crate::check_preflight::check_interrupted;
use crate::check_types::{InterrogationResult, SelectedExpectation};
use crate::config_types::AgentConfig;
use crate::evaluator_turn::{is_model_technical_failure, model_label};
use crate::evaluator_types::{EvaluatorError, EvaluatorRunner};
use crate::logging::DiagnosticLogWriter;
use serde_json::json;

pub(crate) fn interrogate_expectation_with_model_fallbacks<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    expectation: &SelectedExpectation,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    enforced_scope: &[String],
) -> Result<InterrogationResult, String> {
    run_with_model_fallbacks(
        &runtime.config.agent,
        state,
        diagnostic_log,
        Some(&expectation.id),
        |state, diagnostic_log, model| {
            interrogate_expectation_with_model(
                runtime,
                expectation,
                runner,
                diagnostic_log,
                state,
                enforced_scope,
                model,
            )
        },
    )
}

pub(crate) fn run_with_model_fallbacks<T>(
    agent: &AgentConfig,
    state: &mut InterrogationState,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
    mut attempt: impl FnMut(
        &mut InterrogationState,
        &mut Option<&mut DiagnosticLogWriter>,
        Option<&str>,
    ) -> Result<T, EvaluatorError>,
) -> Result<T, String> {
    let mut failures = Vec::new();
    let models = state.available_models(agent);
    for (model_index, model) in models.iter().enumerate() {
        if check_interrupted() {
            return Err("interrupted".to_string());
        }
        match attempt(state, diagnostic_log, model.as_deref()) {
            Ok(result) => return Ok(result),
            Err(err) if is_model_technical_failure(&err) => {
                let next_model = models.get(model_index + 1);
                if next_model.is_some() {
                    // Fallback attempts are the technical-failure exception to
                    // normal same-scope thread reuse. The failing model may
                    // have caused the app server to retire every live thread,
                    // so the next model must start from fresh sessions.
                    state.clear_thread_sessions();
                    state.mark_model_unavailable(model.as_deref());
                }
                write_model_fallback_events(
                    diagnostic_log,
                    expectation_id,
                    model.as_deref(),
                    next_model.and_then(Option::as_deref),
                    err.message_str(),
                )?;
                failures.push(format!(
                    "{}: {}",
                    model_label(model.as_deref()),
                    err.message_str()
                ));
            }
            Err(err) => return Err(err.to_string()),
        }
    }
    Err(format!(
        "all evaluator models failed: {}",
        failures.join("; ")
    ))
}

pub(crate) fn write_model_fallback_events(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
    model: Option<&str>,
    next_model: Option<&str>,
    error: &str,
) -> Result<(), String> {
    let Some(writer) = diagnostic_log.as_deref_mut() else {
        return Ok(());
    };
    writer
        .write_event(
            "warn",
            "model.failure",
            &[
                ("id", json!(expectation_id)),
                ("model", json!(model_label(model))),
                ("error", json!(error)),
            ],
        )
        .map_err(|err| err.to_string())?;
    if let Some(next_model) = next_model {
        writer
            .write_event(
                "warn",
                "model.fallback",
                &[
                    ("id", json!(expectation_id)),
                    ("from", json!(model_label(model))),
                    ("to", json!(model_label(Some(next_model)))),
                    ("reason", json!(error)),
                ],
            )
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}
