use super::{
    deserialize_non_null_configured_value, deserialize_optional_scalar_string,
    deserialize_optional_string, AgentConfig, ConfiguredValue, CooldownConfig, ExpectationTo,
    QScopeConfig, RawExpectationCommonConfig, RawExpectationSettings,
    RawGitBackedExpectationConfig,
};
use serde::Deserialize;

/// Yields selected preset names from highest to lowest field precedence.
pub(crate) fn preset_names_in_precedence_order(
    selected_presets: &str,
) -> impl Iterator<Item = &str> {
    selected_presets.rsplit('+').map(str::trim)
}

#[derive(Debug, Deserialize, Clone, Default)]
pub(crate) struct RawPresetConfig {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub(crate) q: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_scalar_string")]
    pub(crate) a: Option<String>,
    #[serde(default, deserialize_with = "deserialize_non_null_configured_value")]
    pub(crate) to: ConfiguredValue<ExpectationTo>,
    #[serde(default)]
    pub(crate) rank: ConfiguredValue<i64>,
    #[serde(default)]
    // Human-authored expectation context data inherited by expectation items.
    // Despite the config key name, this is not an implementation-owned
    // evaluator-agent instruction source; only resource templates under
    // `resources/prompts/` decide how to embed it.
    #[serde(rename = "instructions")]
    pub(crate) question_context: ConfiguredValue<String>,
    #[serde(default)]
    #[serde(rename = "diff-from")]
    pub(crate) diff_from: ConfiguredValue<String>,
    #[serde(default)]
    pub(crate) target: ConfiguredValue<String>,
    #[serde(default)]
    pub(crate) cooldown: ConfiguredValue<CooldownConfig>,
    #[serde(default)]
    #[serde(rename = "q-scope")]
    pub(crate) q_scope: ConfiguredValue<QScopeConfig>,
    #[serde(default)]
    pub(crate) preset: Option<String>,
    #[serde(default)]
    pub(crate) models: ConfiguredValue<Vec<String>>,
    #[serde(default)]
    pub(crate) thinking: ConfiguredValue<String>,
    #[serde(default)]
    pub(crate) ignore: ConfiguredValue<Vec<String>>,
    #[serde(default)]
    pub(crate) plugins: ConfiguredValue<Vec<String>>,
}

impl RawPresetConfig {
    pub(crate) fn expectation_common(&self) -> RawExpectationCommonConfig {
        RawExpectationCommonConfig {
            question_context: self.question_context.clone(),
            git_backed: RawGitBackedExpectationConfig {
                diff_from: self.diff_from.clone(),
                target: self.target.clone(),
                cooldown: self.cooldown.clone(),
                q_scope: self.q_scope.clone(),
            },
            to: self.to.clone(),
            rank: self.rank.clone(),
            settings: RawExpectationSettings {
                preset: None,
                models: self.models.clone(),
                thinking: self.thinking.clone(),
                ignore: self.ignore.clone(),
                plugins: self.plugins.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ResolvedPresetConfig {
    pub(crate) q: Option<String>,
    pub(crate) a: Option<String>,
    pub(crate) common: RawExpectationCommonConfig,
}

impl ResolvedPresetConfig {
    pub(crate) fn agent_config(&self) -> AgentConfig {
        let mut agent = AgentConfig::implementation_default();
        self.common.settings.apply_to_agent(&mut agent);
        agent
    }
}
