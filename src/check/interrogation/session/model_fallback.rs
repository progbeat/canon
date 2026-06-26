use crate::check::core::{InterrogationResult, SelectedExpectation};
use crate::check::interrogation::interrogate_expectation_with_model;
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::config_types::AgentConfig;
use crate::evaluator::{
    is_model_technical_failure, model_label, EvaluatorError, EvaluatorProgress, EvaluatorRunner,
};
use crate::logs::DiagnosticLogWriter;
use crate::platform::check_interrupted;
use crate::xpec_state::XpecStateCache;
use serde_json::json;

pub(crate) struct ModelFallbackInterrogation<'a> {
    pub(crate) runtime: &'a CheckRuntime<'a>,
    pub(crate) expectation: &'a SelectedExpectation,
    pub(crate) enforced_scope: &'a [String],
    pub(crate) progress: Option<&'a EvaluatorProgress>,
}

pub(crate) fn interrogate_expectation_with_model_fallbacks<R: EvaluatorRunner>(
    interrogation: ModelFallbackInterrogation<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    xpec_state: &mut XpecStateCache,
) -> Result<InterrogationResult, String> {
    let ModelFallbackInterrogation {
        runtime,
        expectation,
        enforced_scope,
        progress,
    } = interrogation;
    run_with_model_fallbacks(
        &expectation.agent,
        state,
        diagnostic_log,
        Some(&expectation.id),
        progress,
        |state, diagnostic_log, model| {
            interrogate_expectation_with_model(
                runtime,
                expectation,
                runner,
                diagnostic_log,
                state,
                xpec_state,
                enforced_scope,
                model,
                progress,
            )
        },
    )
}

pub(crate) fn run_with_model_fallbacks<T>(
    agent: &AgentConfig,
    state: &mut InterrogationRunState,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
    progress: Option<&EvaluatorProgress>,
    mut attempt: impl FnMut(
        &mut InterrogationRunState,
        &mut Option<&mut DiagnosticLogWriter>,
        Option<&str>,
    ) -> Result<T, EvaluatorError>,
) -> Result<T, String> {
    let mut failures = Vec::new();
    let models = state.models_in_retry_order(agent);
    for (model_index, model) in models.iter().enumerate() {
        if check_interrupted() {
            return Err("interrupted".to_string());
        }
        match attempt(state, diagnostic_log, model.as_deref()) {
            Ok(result) => return Ok(result),
            Err(err) if is_model_technical_failure(&err) => {
                let next_model = models.get(model_index + 1);
                if next_model.is_some() {
                    if let Some(progress) = progress {
                        // A fallback clears live sessions, so the next attempt
                        // may begin with thread/start. Record the canon `⇄`
                        // marker before that control message can happen.
                        progress.record_model_fallback_started();
                    }
                    // `progress` is present for selected-expectation result
                    // timelines, where this fallback maps to the `⇄` marker.
                    // Query mode reuses fallback behavior but has no public
                    // result-entry timeline at all.
                    // Fallback attempts are the technical-failure exception to
                    // normal model/visible-context thread reuse. The failing
                    // model may have caused the app server to retire every live
                    // thread, so the next model must start from fresh sessions.
                    state.clear_thread_sessions();
                }
                write_model_fallback_events(
                    diagnostic_log,
                    expectation_id,
                    model.as_deref(),
                    next_model.and_then(Option::as_deref),
                    err.message_str(),
                );
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
) {
    let Some(writer) = diagnostic_log.as_deref_mut() else {
        return;
    };
    // Fallback decisions must remain driven by evaluator errors; diagnostic
    // event write failures are non-functional observability failures.
    let _ = writer.write_event(
        "warn",
        "model.failure",
        &[
            ("id", json!(expectation_id)),
            ("model", json!(model)),
            ("error", json!(error)),
        ],
    );
    if let Some(next_model) = next_model {
        let _ = writer.write_event(
            "warn",
            "model.fallback",
            &[
                ("id", json!(expectation_id)),
                ("from", json!(model)),
                ("to", json!(next_model)),
                ("reason", json!(error)),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::run_with_model_fallbacks;
    use crate::check::interrogation::state::InterrogationRunState;
    use crate::config_types::AgentConfig;
    use crate::evaluator::{EvaluatorError, EvaluatorFailureKind, EvaluatorProgress};
    use crate::logs::DiagnosticLogWriter;

    #[test]
    fn model_fallback_tries_configured_models_in_order() {
        let agent = AgentConfig {
            models: vec!["first".to_string(), "second".to_string()],
            ..AgentConfig::default()
        };
        let mut state = InterrogationRunState::new(true).unwrap();
        let progress = EvaluatorProgress::new();
        let mut attempts = Vec::new();
        let mut diagnostic_log: Option<&mut DiagnosticLogWriter> = None;

        let result = run_with_model_fallbacks(
            &agent,
            &mut state,
            &mut diagnostic_log,
            Some("test-expectation"),
            Some(&progress),
            |_state, _diagnostic_log, model| {
                attempts.push(model.map(str::to_string));
                if attempts.len() == 1 {
                    return Err(EvaluatorError::failure(
                        EvaluatorFailureKind::ModelUnavailable,
                        "first model unavailable",
                    ));
                }
                Ok("ok")
            },
        )
        .unwrap();

        assert_eq!(result, "ok");
        assert_eq!(
            attempts,
            vec![Some("first".to_string()), Some("second".to_string())]
        );
    }
}
