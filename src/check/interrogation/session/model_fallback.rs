use super::thread::interrogate_expectation_answer_with_model;
use super::InterrogationSession;
use crate::check::core::{InterrogationAnswer, InterrogationResult, ResolvedExpectation};
use crate::check::interrogation::interrogation_result_from_answer;
use crate::check::interrogation::state::CheckRuntime;
use crate::check::interrogation::InterrogationTurnKind;
use crate::config_types::AgentConfig;
use crate::evaluator::{
    evaluator_models, is_technical_failure, EvaluatorAttemptReason, EvaluatorAttemptSequence,
    EvaluatorError, EvaluatorProgress, EvaluatorRunner,
};
use crate::logs::DiagnosticLogWriter;
use crate::platform::process::check_interrupted;
use crate::xpec_state::XpecStateCache;

mod events;

pub(crate) use events::write_model_fallback_events;

pub(crate) struct ModelFallbackInterrogation<'a> {
    pub(crate) runtime: &'a CheckRuntime<'a>,
    pub(crate) expectation: &'a ResolvedExpectation,
    pub(crate) enforced_scope: &'a [String],
    pub(crate) turn_kind: InterrogationTurnKind,
    pub(crate) progress: Option<&'a EvaluatorProgress>,
}

pub(crate) struct ModelAttempt<'a> {
    pub(crate) model: Option<&'a str>,
    pub(crate) attempt_reason: EvaluatorAttemptReason,
    pub(crate) attempt_sequence: &'a mut EvaluatorAttemptSequence,
}

pub(crate) trait ModelFallbackOutput: Sized {
    fn from_answer(
        expectation: &ResolvedExpectation,
        diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
        answer: InterrogationAnswer,
    ) -> Result<Self, EvaluatorError>;

    fn interrogate_with_model<R: EvaluatorRunner>(
        interrogation: &ModelFallbackInterrogation<'_>,
        runner: &mut R,
        diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
        visible_tree_oid_cache: &mut crate::git::VisibleTreeOidCache,
        interrogation_session: &mut InterrogationSession,
        xpec_state: &mut XpecStateCache,
        attempt: ModelAttempt<'_>,
    ) -> Result<Self, EvaluatorError> {
        let answer = interrogate_expectation_answer_with_model(
            interrogation,
            runner,
            diagnostic_log,
            visible_tree_oid_cache,
            interrogation_session,
            xpec_state,
            attempt,
        )?;
        Self::from_answer(interrogation.expectation, diagnostic_log, answer)
    }
}

impl ModelFallbackOutput for InterrogationResult {
    fn from_answer(
        expectation: &ResolvedExpectation,
        diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
        answer: InterrogationAnswer,
    ) -> Result<Self, EvaluatorError> {
        interrogation_result_from_answer(expectation, diagnostic_log, answer)
    }
}

impl ModelFallbackOutput for InterrogationAnswer {
    fn from_answer(
        _expectation: &ResolvedExpectation,
        _diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
        answer: InterrogationAnswer,
    ) -> Result<Self, EvaluatorError> {
        Ok(answer)
    }
}

pub(crate) fn interrogate_with_model_fallbacks<T: ModelFallbackOutput, R: EvaluatorRunner>(
    interrogation: ModelFallbackInterrogation<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    visible_tree_oid_cache: &mut crate::git::VisibleTreeOidCache,
    interrogation_session: &mut InterrogationSession,
    xpec_state: &mut XpecStateCache,
) -> Result<T, EvaluatorError> {
    interrogation
        .turn_kind
        .record_turn_started_progress_marker(interrogation.progress);
    let mut attempt_sequence = EvaluatorAttemptSequence::default();
    run_with_model_fallbacks(
        &interrogation.expectation.agent,
        diagnostic_log,
        interrogation.expectation.configured_id(),
        interrogation.progress,
        |diagnostic_log, model, attempt_reason| {
            T::interrogate_with_model(
                &interrogation,
                runner,
                diagnostic_log,
                visible_tree_oid_cache,
                interrogation_session,
                xpec_state,
                ModelAttempt {
                    model,
                    attempt_reason,
                    attempt_sequence: &mut attempt_sequence,
                },
            )
        },
    )
}

pub(crate) fn run_with_model_fallbacks<T>(
    agent: &AgentConfig,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
    progress: Option<&EvaluatorProgress>,
    mut attempt: impl FnMut(
        &mut Option<&mut DiagnosticLogWriter>,
        Option<&str>,
        EvaluatorAttemptReason,
    ) -> Result<T, EvaluatorError>,
) -> Result<T, EvaluatorError> {
    let mut failures = Vec::new();
    let mut diagnostic_log_error = None;
    let models = evaluator_models(agent);
    for (model_index, model) in models.iter().enumerate() {
        if check_interrupted() {
            return Err(EvaluatorError::interrupted());
        }
        // A model fallback is another attempt to acquire this logical turn's
        // response, not another policy turn. Its distinct timeline event is
        // recorded below as `⇄`; the turn-start event was recorded once by
        // `interrogate_with_model_fallbacks` before entering this attempt loop.
        let attempt_reason = if model_index == 0 {
            EvaluatorAttemptReason::Initial
        } else {
            EvaluatorAttemptReason::ModelFallback
        };
        match attempt(diagnostic_log, model.as_deref(), attempt_reason) {
            // [Eg] A model attempt acquired the logical turn's response. A
            // prior observability failure cannot replace that successful
            // evaluator result; the command-level log lifecycle reports
            // deferred writer failures after required evaluation effects.
            Ok(result) => return Ok(result),
            Err(err) if is_technical_failure(&err) => {
                // [Eg] `ask_thread_turn` returns a technical error only after
                // classifying and exhausting any retry applicable to the
                // current model. The thread lifecycle also invalidates failed
                // threads before returning it. This outer layer therefore
                // owns only the later-model fallback and must not duplicate
                // current-model retry or thread ownership.
                let next_model = models.get(model_index + 1);
                if next_model.is_some() {
                    if let Some(progress) = progress {
                        // Record the canon `⇄` marker before the next model can
                        // start its evaluator thread.
                        progress.record_model_fallback_started();
                    }
                    // `progress` is present for selected-expectation and
                    // `canon ask` timelines, where this fallback maps to the
                    // `⇄` marker.
                }
                if let Err(err) = write_model_fallback_events(
                    diagnostic_log,
                    expectation_id,
                    model.as_deref(),
                    next_model.and_then(Option::as_deref),
                    err.message_str(),
                ) {
                    diagnostic_log_error.get_or_insert_with(|| err.to_string());
                }
                failures.push(format!(
                    "{}: {}",
                    model.as_deref().unwrap_or("<default>"),
                    err.message_str()
                ));
            }
            Err(err) => return Err(err),
        }
    }
    let mut fallback_error = format!("all evaluator models failed: {}", failures.join("; "));
    if let Some(err) = diagnostic_log_error {
        fallback_error.push_str("; failed to record model fallback events: ");
        fallback_error.push_str(&err);
    }
    Err(EvaluatorError::message(fallback_error))
}

#[cfg(test)]
mod tests {
    use super::run_with_model_fallbacks;
    use crate::config_types::AgentConfig;
    use crate::evaluator::{
        EvaluatorAttemptReason, EvaluatorError, EvaluatorFailureKind, EvaluatorProgress,
    };
    use crate::logs::DiagnosticLogWriter;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: qv,gN
    fn model_fallback_tries_configured_models_in_order() {
        let agent = AgentConfig {
            models: vec!["first".to_string(), "second".to_string()],
            ..AgentConfig::default()
        };
        let progress = EvaluatorProgress::new();
        let mut attempts = Vec::new();
        let mut diagnostic_log: Option<&mut DiagnosticLogWriter> = None;

        let result = run_with_model_fallbacks(
            &agent,
            &mut diagnostic_log,
            Some("test-expectation"),
            Some(&progress),
            |_diagnostic_log, model, reason| {
                attempts.push((model.map(str::to_string), reason));
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
            vec![
                (Some("first".to_string()), EvaluatorAttemptReason::Initial),
                (
                    Some("second".to_string()),
                    EvaluatorAttemptReason::ModelFallback,
                ),
            ]
        );
    }

    #[test] // xpec: Eg
    fn model_fallback_log_error_does_not_replace_later_success() {
        let root = temp_git_repo("model-fallback-log-error");
        git(&root, &["config", "canon.logs.maxSize", "1"]);
        let mut writer = DiagnosticLogWriter::create(&root).unwrap();
        let mut diagnostic_log = Some(&mut writer);
        let agent = AgentConfig {
            models: vec!["first".to_string(), "second".to_string()],
            ..AgentConfig::default()
        };
        let mut attempts = Vec::new();

        let result = run_with_model_fallbacks(
            &agent,
            &mut diagnostic_log,
            Some("test-expectation"),
            None,
            |_diagnostic_log, model, _reason| {
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
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: u
    fn interruption_remains_typed_and_is_not_retried() {
        let agent = AgentConfig {
            models: vec!["first".to_string(), "second".to_string()],
            ..AgentConfig::default()
        };
        let mut diagnostic_log: Option<&mut DiagnosticLogWriter> = None;
        let mut attempts = 0;

        let err = run_with_model_fallbacks(
            &agent,
            &mut diagnostic_log,
            Some("test-expectation"),
            None,
            |_diagnostic_log, _model, _reason| {
                attempts += 1;
                Err::<(), _>(EvaluatorError::interrupted())
            },
        )
        .unwrap_err();

        assert_eq!(err.kind(), Some(EvaluatorFailureKind::Interrupted));
        assert_eq!(attempts, 1);
    }

    fn temp_git_repo(name: &str) -> PathBuf {
        let root = temp_root(name);
        git(&root, &["init"]);
        root
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        // xpec: qv
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("canon-test-{name}-{}-{unique}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }
}
