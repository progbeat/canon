mod error;
mod prepared;

pub(super) use error::AskQueryError;
use prepared::{run_started_ask_query_command, StartedAskQueryCommand};

use crate::check::command::{GitBackedCheckResources, TokenUsageSummary};
use crate::check::interrogation::state::CheckTreeContext;
use crate::check::interrogation::{
    write_query_lifecycle_finish_event, write_query_lifecycle_start_event,
};
use crate::check::CheckRunCaches;
use crate::config_types::CheckConfig;
use crate::git::TreeSource;
use crate::logs::DiagnosticLogWriter;
use std::path::Path;

pub(super) struct AskQueryCommand<'a> {
    pub(crate) root: &'a Path,
    pub(crate) question: &'a str,
    pub(crate) config: &'a CheckConfig,
    pub(crate) tree_source: Option<TreeSource>,
    pub(crate) tree_context: Option<CheckTreeContext>,
    pub(crate) resources: Option<GitBackedCheckResources>,
    pub(crate) in_place: bool,
    pub(crate) no_sandbox: bool,
    pub(crate) diagnostic_log: Option<DiagnosticLogWriter>,
    pub(crate) check_caches: &'a mut CheckRunCaches,
    pub(crate) token_usage_summary: &'a mut TokenUsageSummary,
}

pub(super) fn run_ask_query_command(command: AskQueryCommand<'_>) -> Result<(), AskQueryError> {
    let AskQueryCommand {
        root,
        question,
        config,
        tree_source,
        tree_context,
        resources,
        in_place,
        no_sandbox,
        diagnostic_log,
        check_caches,
        token_usage_summary,
    } = command;
    let mut diagnostic_log = diagnostic_log;
    if let Some(writer) = diagnostic_log.as_mut() {
        // [w,l] Ask event production remains unconditional even though its
        // temporary-query writer retains the rendered events only in memory.
        write_query_lifecycle_start_event(writer).map_err(|err| err.to_string())?;
    }
    let result = run_started_ask_query_command(StartedAskQueryCommand {
        root,
        question,
        config,
        tree_source,
        tree_context,
        resources,
        in_place,
        no_sandbox,
        diagnostic_log: diagnostic_log.as_mut(),
        check_caches,
        token_usage_summary,
    });
    let finish_error = result.as_ref().err().map(ToString::to_string);
    if let Some(writer) = diagnostic_log.as_mut() {
        write_query_lifecycle_finish_event(writer, finish_error.as_deref())
            .map_err(|err| err.to_string())?;
    }
    result
}
