use super::{string_at_path, value_at_path, AppServerEventKind, AppServerMessage};
use crate::evaluator::{EvaluatorError, EvaluatorFailureKind};
use serde::Deserialize;
use serde_json::{json, Value};

pub(crate) fn app_server_failure_kind(error: &Value) -> EvaluatorFailureKind {
    let code = serde_json::from_value::<AppServerErrorFields>(error.clone())
        .ok()
        .and_then(AppServerErrorFields::code);
    code.as_deref()
        .map(app_server_failure_kind_from_code)
        .unwrap_or(EvaluatorFailureKind::UnknownAppServer)
}

pub(crate) fn app_server_failure_from_value(method: &str, error: &Value) -> EvaluatorError {
    let failure = format!("app-server {} failed: {}", method, error);
    EvaluatorError::failure(app_server_failure_kind(error), failure)
}

pub(crate) fn app_server_failure_kind_from_code(code: &str) -> EvaluatorFailureKind {
    match code {
        "usageLimitExceeded" | "usage_limit_exceeded" => EvaluatorFailureKind::UsageLimit,
        "rateLimitExceeded" | "rate_limit_exceeded" => EvaluatorFailureKind::RateLimit,
        "modelUnavailable" | "model_unavailable" => EvaluatorFailureKind::ModelUnavailable,
        "contextWindowExceeded" | "context_window_exceeded" | "context_length_exceeded" => {
            EvaluatorFailureKind::ContextWindow
        }
        _ => EvaluatorFailureKind::UnknownAppServer,
    }
}

#[derive(Deserialize)]
struct AppServerErrorFields {
    code: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "codexErrorInfo")]
    codex_error_info: Option<String>,
}

impl AppServerErrorFields {
    fn code(self) -> Option<String> {
        self.code.or(self.kind).or(self.codex_error_info)
    }
}

pub(crate) fn app_server_error_value(message: &AppServerMessage<'_>) -> Option<Value> {
    let method = message.method?;
    if message.kind == AppServerEventKind::Error && app_server_error_will_retry(message) {
        return None;
    }
    if !matches!(
        message.kind,
        AppServerEventKind::Error | AppServerEventKind::TurnFailed | AppServerEventKind::TurnError
    ) {
        if message.kind == AppServerEventKind::TurnCompleted
            && message
                .params
                .and_then(|params| string_at_path(params, &["turn", "status"]))
                == Some("failed")
        {
            return message
                .params?
                .get("turn")?
                .get("error")
                .cloned()
                .or_else(|| Some(json!({ "message": "turn failed" })));
        }
        return None;
    }
    message
        .params
        .and_then(|params| value_at_path(params, &["error"]))
        .or(message.error)
        .cloned()
        .or_else(|| {
            message
                .params
                .and_then(|params| string_at_path(params, &["message"]))
                .map(message_error_value)
        })
        .or_else(|| string_at_path(message.raw, &["message"]).map(message_error_value))
        .or_else(|| Some(message_error_value(method)))
}

fn app_server_error_will_retry(message: &AppServerMessage<'_>) -> bool {
    message
        .params
        .and_then(|params| value_at_path(params, &["willRetry"]))
        .or_else(|| {
            message
                .params
                .and_then(|params| value_at_path(params, &["will_retry"]))
        })
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub(crate) fn message_error_value(message: &str) -> Value {
    json!({ "message": message })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: qv
    fn retrying_app_server_error_notification_is_not_final_error() {
        let message = json!({
            "method": "error",
            "params": {
                "willRetry": true,
                "error": {
                    "message": "Reconnecting... 5/5",
                    "additionalDetails": "request timed out"
                }
            }
        });
        let message = crate::app::protocol::app_server_message(&message).unwrap();

        assert_eq!(app_server_error_value(&message), None);
    }

    #[test] // xpec: qv
    fn non_retry_app_server_error_notification_is_final_error() {
        let message = json!({
            "method": "error",
            "params": {
                "willRetry": false,
                "error": {
                    "message": "request failed"
                }
            }
        });
        let message = crate::app::protocol::app_server_message(&message).unwrap();

        assert_eq!(
            app_server_error_value(&message),
            Some(json!({ "message": "request failed" }))
        );
    }
}
