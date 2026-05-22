use super::*;

#[test]
fn completed_agent_message_text_is_turn_text_fallback() {
    let message = json!({
        "method": "item/completed",
        "params": {
            "item": {
                "role": "assistant",
                "content": [
                    { "type": "output_text", "text": "ANSWER: yes\n" },
                    { "type": "tool_result", "metadata": { "text": "ignored" } },
                    { "type": "output_text", "text": "EVIDENCE:\nok\nSCOPE: [\".\"]" }
                ]
            }
        }
    });
    let mut completed_text = String::new();
    append_completed_agent_text(&message, &mut completed_text);
    assert_eq!(
        turn_text(String::new(), completed_text),
        "ANSWER: yes\nEVIDENCE:\nok\nSCOPE: [\".\"]"
    );
    assert_eq!(
        turn_text("ANSWER: no".to_string(), "ANSWER: yes".to_string()),
        "ANSWER: no"
    );
}

#[test]
fn app_server_error_message_is_extracted() {
    let message = json!({
        "method": "error",
        "params": {
            "error": {
                "message": "You've hit your usage limit for GPT-5.3-Codex-Spark."
            }
        }
    });
    assert_eq!(
        app_server_error_message(&message).unwrap(),
        "You've hit your usage limit for GPT-5.3-Codex-Spark."
    );

    let turn_completed = json!({
        "method": "turn/completed",
        "params": {
            "turn": {
                "status": "failed",
                "error": {
                    "code": "modelUnavailable",
                    "message": "model unavailable"
                }
            }
        }
    });
    assert_eq!(
        app_server_error_message(&turn_completed).unwrap(),
        "model unavailable"
    );
    assert_eq!(
        app_server_failure_from_value(
            "turn/run",
            &app_server_error_value(&turn_completed).unwrap()
        )
        .kind(),
        Some(EvaluatorFailureKind::ModelUnavailable)
    );
}

#[test]
fn evaluator_failure_classification_uses_typed_error() {
    let usage_limit = EvaluatorError::failure(
        EvaluatorFailureKind::UsageLimit,
        "app-server turn/start failed: usageLimitExceeded",
    );
    assert!(is_model_technical_failure(&usage_limit));
    assert!(session_failure_invalidates_thread(&usage_limit));
    assert!(!is_model_technical_failure(&EvaluatorError::message(
        "evaluator response scope [\"context window\"] widens enforced scope [\".\"]"
    )));

    let timeout = EvaluatorError::failure(
        EvaluatorFailureKind::TurnTimeout,
        "app-server turn/run timed out after 300 seconds without progress",
    );
    assert_eq!(timeout.kind(), Some(EvaluatorFailureKind::TurnTimeout));
    assert!(session_failure_invalidates_thread(&timeout));
    assert_eq!(
        timeout.message_str(),
        "app-server turn/run timed out after 300 seconds without progress"
    );
    let app_server_usage_limit = app_server_failure_from_value(
        "turn/run",
        &json!({
            "codexErrorInfo": "usageLimitExceeded",
            "message": "You've hit your usage limit for GPT-5.3-Codex-Spark."
        }),
    );
    assert_eq!(
        app_server_usage_limit.kind(),
        Some(EvaluatorFailureKind::UsageLimit)
    );
    let unknown_app_server_error = app_server_failure_from_value(
        "turn/run",
        &json!({
            "codexErrorInfo": "newTransientServerCode",
            "message": "new server-side failure"
        }),
    );
    assert_eq!(
        unknown_app_server_error.kind(),
        Some(EvaluatorFailureKind::UnknownAppServer)
    );
    assert!(is_model_technical_failure(&unknown_app_server_error));
    assert!(session_failure_invalidates_thread(
        &unknown_app_server_error
    ));
    let untyped_message = app_server_failure_from_message(
        "turn/run",
        "You've hit your usage limit for GPT-5.3-Codex-Spark.",
    );
    assert_eq!(untyped_message.kind(), None);
}

#[test]
fn app_server_message_rejects_malformed_envelopes() {
    assert!(app_server_message(&json!({"id": 1, "result": {}})).is_ok());
    assert!(app_server_message(&json!({"method": "turn/started", "params": {}})).is_ok());

    let err = app_server_message(&json!({"params": {}})).unwrap_err();
    assert!(err.contains("missing both id and method"));
}

#[test]
fn turn_timeout_resets_after_app_server_progress() {
    let first_activity = Instant::now();
    let before_timeout = first_activity + Duration::from_secs(APP_SERVER_TURN_TIMEOUT_SECS - 1);
    let at_timeout = first_activity + Duration::from_secs(APP_SERVER_TURN_TIMEOUT_SECS);

    assert!(!turn_idle_timed_out(first_activity, before_timeout));
    assert!(turn_idle_timed_out(first_activity, at_timeout));

    let later_activity = at_timeout;
    let later_before_timeout =
        later_activity + Duration::from_secs(APP_SERVER_TURN_TIMEOUT_SECS - 1);
    assert!(!turn_idle_timed_out(later_activity, later_before_timeout));
}

#[test]
fn token_usage_update_keeps_raw_app_server_usage() {
    let message = json!({
        "method": "thread/tokenUsage/updated",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "tokenUsage": {
                "total": {
                    "totalTokens": 100000,
                    "inputTokens": 90000,
                    "cachedInputTokens": 400000,
                    "outputTokens": 10000,
                    "reasoningOutputTokens": 5000
                },
                "last": {
                    "totalTokens": 69748,
                    "inputTokens": 63574,
                    "cachedInputTokens": 361216,
                    "outputTokens": 6174,
                    "reasoningOutputTokens": 2911
                },
                "modelContextWindow": 200000
            }
        }
    });
    let update = token_usage_update(&message).unwrap();
    assert_eq!(update.thread_id, "thread-1");
    assert_eq!(update.turn_id, "turn-1");
    assert_eq!(update.sequence, 0);
    assert_eq!(update.token_usage, message["params"]["tokenUsage"]);
    assert_eq!(
        render_token_usage_summary(update.last_usage),
        "Token usage: total=69748 input=63574 (+ 361216 cached) output=6174 (reasoning 2911)"
    );
    assert_eq!(
        render_token_usage_summary(TokenUsage::default()),
        "Token usage: total=0 input=0 (+ 0 cached) output=0 (reasoning 0)"
    );
}

#[test]
fn reference_token_cost_uses_fixed_reference_weights() {
    let cost = reference_token_cost(1_000, 400, 30);

    assert!((cost - 0.00094).abs() < f64::EPSILON);
}

#[test]
#[should_panic(expected = "cached_input_tokens cannot exceed input_tokens")]
fn reference_token_cost_rejects_cached_input_above_total_input() {
    let _ = reference_token_cost(399, 400, 30);
}

#[test]
fn token_usage_reference_cost_treats_cached_input_as_additional_input() {
    let usage = TokenUsage {
        input_tokens: 600,
        cached_input_tokens: 400,
        output_tokens: 30,
        ..TokenUsage::default()
    };

    assert!((usage.reference_token_cost() - 0.00094).abs() < f64::EPSILON);
}

#[test]
fn token_usage_updates_are_kept_ordered_and_summed_by_last_usage() {
    let first = json!({
        "method": "thread/tokenUsage/updated",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "tokenUsage": {
                "total": {
                    "totalTokens": 10,
                    "inputTokens": 7,
                    "cachedInputTokens": 3,
                    "outputTokens": 3,
                    "reasoningOutputTokens": 1
                },
                "last": {
                    "totalTokens": 10,
                    "inputTokens": 7,
                    "cachedInputTokens": 3,
                    "outputTokens": 3,
                    "reasoningOutputTokens": 1
                },
                "modelContextWindow": 200000
            }
        }
    });
    let second = json!({
        "method": "thread/tokenUsage/updated",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "tokenUsage": {
                "total": {
                    "totalTokens": 16,
                    "inputTokens": 11,
                    "cachedInputTokens": 5,
                    "outputTokens": 5,
                    "reasoningOutputTokens": 2
                },
                "last": {
                    "totalTokens": 6,
                    "inputTokens": 4,
                    "cachedInputTokens": 2,
                    "outputTokens": 2,
                    "reasoningOutputTokens": 1
                },
                "modelContextWindow": 200000
            }
        }
    });
    let mut usage_by_turn = BTreeMap::new();
    let mut updates_by_turn = BTreeMap::new();
    record_token_usage_update(
        &mut usage_by_turn,
        &mut updates_by_turn,
        token_usage_update(&first).unwrap(),
    );
    record_token_usage_update(
        &mut usage_by_turn,
        &mut updates_by_turn,
        token_usage_update(&second).unwrap(),
    );

    assert_eq!(
        usage_by_turn["turn-1"],
        TokenUsage {
            total_tokens: 16,
            input_tokens: 11,
            cached_input_tokens: 5,
            output_tokens: 5,
            reasoning_output_tokens: 2
        }
    );
    let updates = &updates_by_turn["turn-1"];
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].sequence, 1);
    assert_eq!(updates[1].sequence, 2);
    assert_eq!(updates[0].token_usage, first["params"]["tokenUsage"]);
    assert_eq!(updates[1].token_usage, second["params"]["tokenUsage"]);
}

#[test]
fn thread_reuse_policy_retires_when_current_carryover_exceeds_50000() {
    let current_inside = TokenUsage {
        input_tokens: 49_000,
        output_tokens: 1_000,
        ..TokenUsage::default()
    };
    let current_too_large = TokenUsage {
        input_tokens: 49_000,
        output_tokens: 1_001,
        ..TokenUsage::default()
    };

    assert_eq!(carryover_tokens(current_inside), 50_000);
    assert!(!thread_reuse_policy_should_retire(carryover_tokens(
        current_inside
    )));
    assert!(thread_reuse_policy_should_retire(carryover_tokens(
        current_too_large
    )));
}

#[test]
fn context_compaction_events_are_kept_raw_and_ordered() {
    let first = json!({
        "method": "item/started",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "item": {
                "id": "item-1",
                "type": "contextCompaction"
            }
        }
    });
    let second = json!({
        "method": "item/completed",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "item": {
                "id": "item-1",
                "type": "contextCompaction",
                "summary": "compacted context"
            }
        }
    });
    let mut events_by_turn = BTreeMap::new();
    record_context_compaction_event(
        &mut events_by_turn,
        context_compaction_event(&first).unwrap(),
    );
    record_context_compaction_event(
        &mut events_by_turn,
        context_compaction_event(&second).unwrap(),
    );

    let events = &events_by_turn["turn-1"];
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].sequence, 1);
    assert_eq!(events[1].sequence, 2);
    assert_eq!(events[0].thread_id, "thread-1");
    assert_eq!(events[0].turn_id, "turn-1");
    assert_eq!(events[0].method, "item/started");
    assert_eq!(events[0].event, first);
    assert_eq!(events[1].event, second);
}

#[test]
fn long_summary_padding_keeps_required_surrounding_spaces() {
    let inner = " 123456789 failed, 456789 errors, 789123 passed, 101112 skipped in 123456789.00s ";
    assert!(inner.len() >= 80);
    let line = pad_summary_line(inner);
    assert!(line.starts_with("= "));
    assert!(line.ends_with(" ="));

    let nearly_wide = format!(" {} ", "x".repeat(77));
    assert_eq!(nearly_wide.len(), 79);
    let line = pad_summary_line(&nearly_wide);
    assert!(line.starts_with("= "));
    assert!(line.ends_with(" ="));
}
