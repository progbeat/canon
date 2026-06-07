use crate::evaluator::EvaluatorError;
use crate::platform::check_interrupted;
use serde_json::{json, Value};
use std::io::Write;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use super::AppServerRunner;

impl AppServerRunner {
    pub(crate) fn maybe_interrupt_turn(
        &mut self,
        interrupted: &mut bool,
        interrupt_sent: &mut bool,
        thread_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<(), EvaluatorError> {
        if !check_interrupted() {
            return Ok(());
        }
        *interrupted = true;
        if *interrupt_sent {
            return Ok(());
        }
        let (Some(thread_id), Some(turn_id)) = (thread_id, turn_id) else {
            return Ok(());
        };
        self.send_turn_interrupt(thread_id, turn_id)?;
        *interrupt_sent = true;
        Ok(())
    }

    pub(crate) fn send_turn_interrupt(
        &mut self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), EvaluatorError> {
        let params = json!({
            "threadId": thread_id,
            "turnId": turn_id
        });
        self.send_json_rpc_request("turn/interrupt", &params, "interrupt")?;
        Ok(())
    }

    pub(crate) fn send_json_rpc_request(
        &mut self,
        method: &str,
        params: &Value,
        operation: &str,
    ) -> Result<u64, EvaluatorError> {
        if check_interrupted() {
            return Err("interrupted".into());
        }
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        writeln!(self.stdin, "{}", request)
            .map_err(|err| format!("failed to write app-server {}: {}", operation, err))?;
        self.stdin
            .flush()
            .map_err(|err| format!("failed to flush app-server {}: {}", operation, err))?;
        Ok(id)
    }

    pub(crate) fn read_message_or_timeout(&mut self) -> Result<Option<Value>, EvaluatorError> {
        match self.messages.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => result.map(Some).map_err(EvaluatorError::message),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                if check_interrupted() {
                    Err("interrupted".into())
                } else {
                    Err(self.app_server_closed_stdout_error())
                }
            }
        }
    }

    fn app_server_closed_stdout_error(&mut self) -> EvaluatorError {
        let mut message = String::from("app-server closed stdout");
        if let Ok(Some(status)) = self.child.try_wait() {
            message.push_str(&format!(" with status {}", status));
        }
        let stderr = self
            .stderr
            .try_iter()
            .collect::<Vec<_>>()
            .join("")
            .trim()
            .to_string();
        if !stderr.is_empty() {
            message.push_str(": ");
            message.push_str(&stderr);
        }
        EvaluatorError::message(message)
    }
}
