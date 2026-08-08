//! Codex app-server sandbox protocol translation for evaluator sessions.
//! These invocation-local request fields own no filesystem state.

use crate::evaluator::{EphemeralEvaluatorThreadPermissionProfile, EvaluatorProcessIsolation};
use serde_json::{json, Value};

/// Invocation-local app-server fields for one ephemeral thread/start request.
pub(super) struct ThreadPermissionSelection {
    pub(super) permissions: Option<EphemeralEvaluatorThreadPermissionProfile>,
    pub(super) sandbox: Option<&'static str>,
}

pub(super) fn thread_permission_selection(
    process_isolation: EvaluatorProcessIsolation,
) -> ThreadPermissionSelection {
    match process_isolation {
        EvaluatorProcessIsolation::CanonManaged => ThreadPermissionSelection {
            permissions: process_isolation.ephemeral_thread_permission_profile(),
            sandbox: None,
        },
        // [l,hQ] This wire value disables only the redundant app-server
        // sandbox because the caller owns process isolation. App-server startup
        // disables the shell independently and exposes only read-only project
        // inspection tools.
        EvaluatorProcessIsolation::ExternallyManaged => ThreadPermissionSelection {
            permissions: None,
            sandbox: Some("danger-full-access"),
        },
    }
}

pub(super) fn turn_sandbox_policy(process_isolation: EvaluatorProcessIsolation) -> Option<Value> {
    match process_isolation {
        EvaluatorProcessIsolation::CanonManaged => None,
        // [l,hQ] This selects the transport inside caller-provided isolation;
        // the independently fixed read-only tool plan does not gain a write
        // capability from this wire-level sandbox policy.
        EvaluatorProcessIsolation::ExternallyManaged => Some(json!({ "type": "dangerFullAccess" })),
    }
}
