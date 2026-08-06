use super::super::{expand_raw_check_config_for_command, CheckConfigExpansionOptions};
use crate::config_types::{
    CheckConfig, Cooldown, ExpectationTarget, ExpectationTo, QScope, RawCheckConfig,
    DEFAULT_DIFF_FROM,
};

fn expand_raw_check_config(raw: RawCheckConfig) -> Result<CheckConfig, String> {
    expand_raw_check_config_for_command(raw, CheckConfigExpansionOptions::default())
}

mod defaults;
mod precedence;
