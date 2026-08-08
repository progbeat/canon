mod failure;
mod prepare;
mod run;
mod trailer;

pub(crate) use prepare::{
    prepare_git_backed_check_execution, resolve_explicit_diff_from_tree_oids,
    GitBackedCheckResources, PrepareGitBackedCheckExecutionOptions,
};
pub(crate) use run::{run_ask_command, run_check_command};
