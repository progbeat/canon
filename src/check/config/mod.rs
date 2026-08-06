pub(super) mod expansion;
mod foreach;
pub(super) mod foreach_paths;
mod in_place;
mod load;
pub(super) mod validation;
pub(crate) mod yaml_include;

pub(crate) use load::{
    collect_check_config, collect_in_place_check_config_with_default_agent_preset,
    is_missing_default_config_error, load_ask_config, load_check_config, load_in_place_ask_config,
};
pub(crate) const CHECK_PATH: &str = ".canon/check.yml";
