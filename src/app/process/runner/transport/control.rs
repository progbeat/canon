use super::super::AppServerRunner;
use crate::app::protocol::{app_server_failure_from_value, app_server_message};
use crate::evaluator::{EvaluatorError, EvaluatorFailureKind};
use crate::platform::process::check_interrupted;
use serde_json::Value;
use std::time::Instant;

use super::timeout::{idle_timeout_elapsed, idle_timeout_error, IdleTimeoutKind};

impl AppServerRunner {
    pub(crate) fn send_control_request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Value, EvaluatorError> {
        let id = self.send_json_rpc_request(method, &params, "request")?;
        let mut last_activity = Instant::now();
        loop {
            if check_interrupted() {
                return Err(EvaluatorError::interrupted());
            }
            let Some(message) = self.read_message_or_timeout()? else {
                let now = Instant::now();
                if idle_timeout_elapsed(last_activity, now) {
                    // Control messages such as initialize and thread/start set
                    // up the app server or a thread; they are not evaluator
                    // work kinds in the per-expectation timeline. If a
                    // non-initial interrogation needs a fresh thread, the
                    // higher-level fallback/retry/verification path records
                    // its dedicated marker before this control message.
                    return Err(idle_timeout_error(method, IdleTimeoutKind::Response));
                }
                continue;
            };
            last_activity = Instant::now();
            let envelope = app_server_message(&message).map_err(|error| {
                EvaluatorError::failure(EvaluatorFailureKind::UnknownAppServer, error)
            })?;
            self.record_app_server_events(&envelope)?;
            if envelope.response_id == Some(id) {
                if let Some(error) = envelope.error {
                    return Err(app_server_failure_from_value(method, error));
                }
                return envelope.result.cloned().ok_or_else(|| {
                    EvaluatorError::message(format!(
                        "app-server {} response missing result",
                        method
                    ))
                });
            }
        }
    }
}
