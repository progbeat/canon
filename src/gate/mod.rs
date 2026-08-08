use crate::cli::{CommandError, ReportedCommandFailure};
use crate::git::{DEFAULT_AGAINST_TREE_ARG, STAGED_TREE_ARG};
use crate::output::write_stderr_line;
use crate::repo_inspection::RepoInspectionCache;
use std::ffi::OsString;
use std::path::Path;

mod decision;
mod regression;
mod staged_paths;

use decision::{decide, Outcome};
use regression::count;

pub(crate) fn run_gate_command(root: &Path, args: &[OsString]) -> Result<(), CommandError> {
    // Gate failure diagnostics have two ownership points: CLI dispatch handles
    // root-resolution failures before this function, and this module handles
    // failures after a project root exists.
    // CLI validation happens before the gate pass/fail decision. These
    // unsupported-option errors are usage errors, not reported gate failures.
    if !args.is_empty() {
        return Err(
            "canon gate does not accept arguments\n▷ Run `canon gate` without arguments.".into(),
        );
    }
    // [Tv] Freeze both symbolic gate inputs during preparation. Every later
    // path and regression inspection consumes these OID-backed sources, so a
    // concurrent index or HEAD update cannot split one gate decision across
    // different repository snapshots.
    let mut repo_cache = RepoInspectionCache::new();
    let baseline_source = gate_command_result(
        repo_cache.resolve_default_against_tree(root, DEFAULT_AGAINST_TREE_ARG),
    )?;
    let staged_source = gate_command_result(repo_cache.resolve_tree_to_oid_source(
        root,
        STAGED_TREE_ARG,
        "--tree",
    ))?;
    let changed_paths = gate_command_result(staged_paths::read(
        &mut repo_cache,
        root,
        &baseline_source,
        &staged_source,
    ))?;
    let unresolved_pass_to_fail_regressions =
        count(root, &mut repo_cache, &baseline_source, &staged_source)?;
    match decide(unresolved_pass_to_fail_regressions, &changed_paths) {
        Outcome::Pass => Ok(()),
        Outcome::RegressionFailure => {
            gate_output_result(write_regression_failure())?;
            Err(CommandError::Reported(ReportedCommandFailure::Gate))
        }
        Outcome::MixedCanonChangeFailure => {
            gate_output_result(write_mixed_canon_change_failure())?;
            Err(CommandError::Reported(ReportedCommandFailure::Gate))
        }
    }
}

fn gate_command_result<T>(result: Result<T, String>) -> Result<T, CommandError> {
    match result {
        Ok(value) => Ok(value),
        Err(err) => {
            gate_output_result(write_gate_error(&err))?;
            Err(CommandError::Reported(ReportedCommandFailure::Gate))
        }
    }
}

fn gate_output_result(result: Result<(), String>) -> Result<(), CommandError> {
    result.map_err(|err| format!("{err}\n▷ Fix stderr output and run `canon gate` again.").into())
}

fn write_gate_error(err: &str) -> Result<(), String> {
    write_stderr_line(&format!("canon gate: {}\n{}", err, gate_error_advice()))
}

fn write_mixed_canon_change_failure() -> Result<(), String> {
    write_stderr_line(
        "canon gate: .canon/** changes must not be mixed with non-.canon changes\n▷ Ask human to handle .canon/ changes.",
    )
}

fn write_regression_failure() -> Result<(), String> {
    // Gate output stays generic by canon: even expectation-related failures
    // are reported without expectation IDs or per-expectation lines. `canon
    // check` is the command that prints individual expectation records.
    write_stderr_line(&format!(
        "canon gate: staged changes regress cached canon results\n{}",
        regression_advice()
    ))
}

pub(crate) fn regression_advice() -> &'static str {
    "▷ Fix staged regressions and run `canon check` again!"
}

pub(crate) fn gate_error_advice() -> &'static str {
    "▷ Fix the gate error and run `canon check` again!"
}
