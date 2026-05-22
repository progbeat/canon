use crate::check_validation::{validate_optional_model, validate_plugin_config_key};

#[test]
fn plugin_keys_must_have_exactly_one_nonempty_separator() {
    assert!(validate_plugin_config_key("plugin@marketplace").is_ok());
    assert!(validate_plugin_config_key("@marketplace").is_err());
    assert!(validate_plugin_config_key("plugin@").is_err());
    assert!(validate_plugin_config_key("plugin@market@extra").is_err());
    assert!(validate_plugin_config_key(" plugin@marketplace").is_err());
    assert!(validate_plugin_config_key("plugin@marketplace ").is_err());
    assert!(validate_plugin_config_key("plugin @marketplace").is_err());
    assert!(validate_plugin_config_key("plugin@ marketplace").is_err());
    assert!(validate_plugin_config_key("foo/bar@marketplace").is_err());
    assert!(validate_plugin_config_key("MyPlugin@marketplace").is_err());
    assert!(validate_plugin_config_key("plugin@openai_curated").is_err());
    assert!(validate_plugin_config_key("foo--bar@marketplace").is_err());
    assert!(validate_plugin_config_key("plugin-1@openai-curated").is_ok());
}

#[test]
fn model_ids_must_not_have_surrounding_whitespace() {
    assert!(validate_optional_model(Some("gpt-5.4-mini"), "agent.model.primary").is_ok());
    assert!(validate_optional_model(Some(" gpt-5.4-mini"), "agent.model.primary").is_err());
    assert!(validate_optional_model(Some("gpt-5.4-mini "), "agent.model.primary").is_err());
    assert!(validate_optional_model(Some("gpt 5.4-mini"), "agent.model.primary").is_err());
    assert!(validate_optional_model(Some("gpt-5.4-mini\u{7}"), "agent.model.primary").is_err());
    assert!(validate_optional_model(Some("gpt-5.4-mini\u{200b}"), "agent.model.primary").is_err());
}
