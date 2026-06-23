use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub(crate) struct CheckConfig {
    pub(crate) version: u32,
    pub(crate) presets: BTreeMap<String, ResolvedPresetConfig>,
    pub(crate) agent: AgentConfig,
    pub(crate) expectations: Vec<Expectation>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawCheckConfig {
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) presets: Option<BTreeMap<String, RawPresetConfig>>,
    #[serde(default)]
    pub(crate) agent: Option<RawLegacyAgentConfig>,
    pub(crate) expectations: Vec<RawExpectationItem>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentConfig {
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default = "default_thinking")]
    pub(crate) thinking: String,
    #[serde(default)]
    pub(crate) ignore: Vec<String>,
    #[serde(default)]
    pub(crate) plugins: Vec<String>,
}

impl AgentConfig {
    pub(crate) fn implementation_default() -> AgentConfig {
        AgentConfig {
            models: Vec::new(),
            thinking: default_thinking(),
            ignore: Vec::new(),
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
pub(crate) struct RawPresetConfig {
    #[serde(default)]
    pub(crate) instructions: Option<String>,
    #[serde(default)]
    #[serde(rename = "diff-from")]
    pub(crate) diff_from: Option<String>,
    #[serde(default)]
    pub(crate) target: Option<String>,
    #[serde(default)]
    pub(crate) cooldown: Option<CooldownConfig>,
    #[serde(default)]
    pub(crate) preset: Option<String>,
    #[serde(default)]
    pub(crate) models: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) thinking: Option<String>,
    #[serde(default)]
    pub(crate) ignore: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) plugins: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ResolvedPresetConfig {
    pub(crate) common: RawExpectationCommonConfig,
}

impl ResolvedPresetConfig {
    pub(crate) fn agent_config(&self) -> AgentConfig {
        let mut agent = AgentConfig::implementation_default();
        let settings = &self.common.settings;
        if let Some(models) = &settings.models {
            agent.models = models.clone();
        }
        if let Some(thinking) = &settings.thinking {
            agent.thinking = thinking.clone();
        }
        if let Some(ignore) = &settings.ignore {
            agent.ignore = ignore.clone();
        }
        if let Some(plugins) = &settings.plugins {
            agent.plugins = plugins.clone();
        }
        agent
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
    pub(crate) ignore: Option<Vec<String>>,
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
    pub(crate) models: Option<Vec<String>>,
    pub(crate) thinking: Option<String>,
    pub(crate) ignore: Option<Vec<String>>,
    pub(crate) plugins: Option<Vec<String>>,
}

pub(crate) fn default_thinking() -> String {
    "low".to_string()
}

pub(crate) const DEFAULT_DIFF_FROM: &str = ":checkpoint";
pub(crate) const AGAINST_TREE_DIFF_FROM: &str = ":against-tree";

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Expectation {
    pub(crate) q: String,
    pub(crate) a: String,
    pub(crate) instructions: String,
    pub(crate) diff_from: String,
    #[serde(default)]
    pub(crate) target: Option<ExpectationTarget>,
    #[serde(default)]
    pub(crate) question_answer_only: bool,
    #[serde(default, skip)]
    pub(crate) agent: AgentConfig,
    #[serde(default)]
    pub(crate) cooldown: Option<CooldownConfig>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ExpectationTarget {
    Project,
    Diff,
}

impl ExpectationTarget {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            ExpectationTarget::Project => "project",
            ExpectationTarget::Diff => "diff",
        }
    }
}

impl std::str::FromStr for ExpectationTarget {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "project" => Ok(ExpectationTarget::Project),
            "diff" => Ok(ExpectationTarget::Diff),
            _ => Err(format!("unsupported target: {}", value)),
        }
    }
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum CooldownConfig {
    Compact(String),
    Mapping(CooldownMappingConfig),
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CooldownMappingConfig {
    #[serde(default)]
    pub(crate) pass: Option<String>,
    #[serde(default)]
    pub(crate) fail: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum RawExpectationItem {
    Explicit(RawExplicitExpectation),
    // The Expectations spec calls both `include` and `path`/`q_template`/`a`
    // forms generator items. Internally they stay split so config expansion can
    // route include recursion separately from per-file question generation.
    Generator(RawGeneratorExpectation),
    Include(RawIncludeExpectation),
}

impl RawExpectationItem {
    pub(crate) fn common_config_mut(&mut self) -> &mut RawExpectationCommonConfig {
        match self {
            RawExpectationItem::Explicit(item) => &mut item.common,
            RawExpectationItem::Generator(item) => &mut item.common,
            RawExpectationItem::Include(item) => &mut item.common,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RawExpectationCommonConfig {
    pub(crate) instructions: Option<String>,
    pub(crate) diff_from: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) cooldown: Option<CooldownConfig>,
    pub(crate) settings: RawExpectationSettings,
}

#[derive(Debug, Clone)]
pub(crate) struct RawExplicitExpectation {
    pub(crate) q: String,
    pub(crate) a: String,
    pub(crate) common: RawExpectationCommonConfig,
}

#[derive(Debug, Clone)]
pub(crate) struct RawGeneratorExpectation {
    // `q_template` is external configuration data for generated expectation
    // questions. The stored value is not a Canon-owned interrogation prompt or
    // instruction template.
    pub(crate) generated_question_format: String,
    pub(crate) path: String,
    pub(crate) a: String,
    pub(crate) common: RawExpectationCommonConfig,
}

#[derive(Debug, Clone)]
pub(crate) struct RawIncludeExpectation {
    pub(crate) include: String,
    pub(crate) common: RawExpectationCommonConfig,
}

#[derive(Debug, Deserialize)]
// Expectation items intentionally omit `deny_unknown_fields`: the expectations
// spec allows extra fields so external IDs or annotations can stay in check files
// without affecting canon's explicit/generator/include expansion.
struct RawExpectationFields {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    q_template: Option<String>,
    #[serde(default)]
    a: Option<String>,
    #[serde(default)]
    instructions: Option<String>,
    #[serde(default)]
    #[serde(rename = "diff-from")]
    diff_from: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    include: Option<String>,
    #[serde(default)]
    cooldown: Option<CooldownConfig>,
    #[serde(default)]
    preset: Option<String>,
    #[serde(default)]
    models: Option<Vec<String>>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    ignore: Option<Vec<String>>,
    #[serde(default)]
    plugins: Option<Vec<String>>,
}

impl<'de> Deserialize<'de> for RawExpectationItem {
    fn deserialize<D>(deserializer: D) -> Result<RawExpectationItem, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let fields = RawExpectationFields::deserialize(deserializer)?;
        RawExpectationItem::from_fields(fields).map_err(serde::de::Error::custom)
    }
}

impl RawExpectationItem {
    fn from_fields(fields: RawExpectationFields) -> Result<RawExpectationItem, &'static str> {
        let RawExpectationFields {
            q,
            q_template,
            a,
            instructions,
            diff_from,
            target,
            path,
            include,
            cooldown,
            preset,
            models,
            thinking,
            ignore,
            plugins,
        } = fields;
        let settings = RawExpectationSettings {
            preset,
            models,
            thinking,
            ignore,
            plugins,
        };
        let common = RawExpectationCommonConfig {
            instructions,
            diff_from,
            target,
            cooldown,
            settings,
        };
        if let Some(include) = include {
            return Ok(RawExpectationItem::Include(RawIncludeExpectation {
                include,
                common,
            }));
        }
        match (q, q_template, path, a) {
            (_, Some(q_template), Some(path), Some(a)) => {
                Ok(RawExpectationItem::Generator(RawGeneratorExpectation {
                    generated_question_format: q_template,
                    path,
                    a,
                    common,
                }))
            }
            (Some(q), _, _, Some(a)) => Ok(RawExpectationItem::Explicit(RawExplicitExpectation {
                q,
                a,
                common,
            })),
            fields => match fields {
                (Some(_), _, _, None) => Err("must contain a"),
                (None, Some(_), None, _) => Err("generator must contain path"),
                (None, Some(_), Some(_), None) => Err("must contain a"),
                (None, None, Some(_), _) => Err("generator must contain q_template"),
                (None, None, None, Some(_)) => Err("must contain q or q_template"),
                (None, None, None, None) => Err("must contain q, q_template, or include"),
                _ => Err("invalid expectation item"),
            },
        }
    }
}
