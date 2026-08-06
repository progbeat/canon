use super::ConfiguredValue;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentConfig {
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default = "default_thinking")]
    pub(crate) thinking: String,
    #[serde(default)]
    pub(crate) ignore: Option<Vec<String>>,
    #[serde(skip)]
    pub(crate) ignore_configured: bool,
    #[serde(default)]
    pub(crate) plugins: Vec<String>,
}

impl AgentConfig {
    pub(crate) fn implementation_default() -> AgentConfig {
        AgentConfig {
            models: Vec::new(),
            thinking: default_thinking(),
            ignore: None,
            ignore_configured: false,
            plugins: Vec::new(),
        }
    }
}

impl Default for AgentConfig {
    fn default() -> AgentConfig {
        AgentConfig::implementation_default()
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawLegacyAgentConfig {
    #[serde(default)]
    pub(crate) model: RawLegacyModelConfig,
    #[serde(default)]
    pub(crate) thinking: Option<String>,
    #[serde(default)]
    pub(crate) ignore: ConfiguredValue<Vec<String>>,
    #[serde(default)]
    pub(crate) plugins: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawLegacyModelConfig {
    #[serde(default)]
    pub(crate) primary: Option<String>,
    #[serde(default)]
    pub(crate) fallbacks: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RawExpectationSettings {
    pub(crate) preset: Option<String>,
    pub(crate) models: ConfiguredValue<Vec<String>>,
    pub(crate) thinking: ConfiguredValue<String>,
    pub(crate) ignore: ConfiguredValue<Vec<String>>,
    pub(crate) plugins: ConfiguredValue<Vec<String>>,
}

impl RawExpectationSettings {
    pub(crate) fn apply_to_agent(&self, agent: &mut AgentConfig) {
        if let Some(models) = &self.models.value {
            agent.models = models.clone();
        }
        if let Some(thinking) = &self.thinking.value {
            agent.thinking = thinking.clone();
        }
        agent.ignore = self.ignore.value.clone();
        agent.ignore_configured = self.ignore.configured;
        if let Some(plugins) = &self.plugins.value {
            agent.plugins = plugins.clone();
        }
    }
}

pub(crate) fn default_thinking() -> String {
    "low".to_string()
}
