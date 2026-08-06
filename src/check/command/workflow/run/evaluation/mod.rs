mod complete;
mod defaults;
mod git_backed;
mod in_place;
mod selection;

use super::super::failure::CheckFailureOutput;
use complete::run_prepared_check;
pub(super) use defaults::{or_fail_with_default_output, prepare_default_failure_output};
pub(super) use git_backed::{run_git_backed_check_command, GitBackedCheckCommandContext};
pub(super) use in_place::{run_in_place_check_command, InPlaceCheckCommandContext};
use selection::PreparedCheckRun;
