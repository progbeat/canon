use crate::config_types::CheckConfig;

pub(super) fn check_config_with_query_preset(
    config: &CheckConfig,
    preset: &str,
) -> Result<CheckConfig, String> {
    let agent = config
        .presets
        .get(preset)
        .cloned()
        .ok_or_else(|| format!("unknown preset: {}", preset))?;
    let mut query_config = config.clone();
    query_config.agent = agent;
    Ok(query_config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_types::AgentConfig;
    use std::collections::BTreeMap;

    fn agent(model: &str, thinking: &str) -> AgentConfig {
        AgentConfig {
            models: vec![model.to_string()],
            thinking: thinking.to_string(),
            ignore: Vec::new(),
            plugins: Vec::new(),
        }
    }

    #[test]
    fn query_preset_overrides_default_agent() {
        let default_agent = agent("default-model", "low");
        let smart_agent = agent("smart-model", "high");
        let mut presets = BTreeMap::new();
        presets.insert("default".to_string(), default_agent.clone());
        presets.insert("smart".to_string(), smart_agent.clone());
        let config = CheckConfig {
            version: 1,
            presets,
            agent: default_agent.clone(),
            expectations: Vec::new(),
        };

        let query_config = check_config_with_query_preset(&config, "smart").unwrap();

        assert_eq!(query_config.agent, smart_agent);
        assert_eq!(config.agent, default_agent);
    }

    #[test]
    fn query_preset_rejects_unknown_name() {
        let default_agent = agent("default-model", "low");
        let mut presets = BTreeMap::new();
        presets.insert("default".to_string(), default_agent.clone());
        let config = CheckConfig {
            version: 1,
            presets,
            agent: default_agent,
            expectations: Vec::new(),
        };

        let err = check_config_with_query_preset(&config, "missing").unwrap_err();

        assert_eq!(err, "unknown preset: missing");
    }
}
