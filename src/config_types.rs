use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckConfig {
    pub(crate) version: u32,
    pub(crate) presets: BTreeMap<String, AgentConfig>,
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
    pub(crate) extends: Option<String>,
    #[serde(default)]
    pub(crate) models: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) thinking: Option<String>,
    #[serde(default)]
    pub(crate) ignore: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) plugins: Option<Vec<String>>,
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

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Expectation {
    pub(crate) q: String,
    pub(crate) a: String,
    #[serde(default, skip)]
    pub(crate) prompt_scope: Vec<String>,
    #[serde(default, skip)]
    pub(crate) agent: AgentConfig,
    #[serde(default)]
    pub(crate) cooldown: Option<String>,
    #[serde(default)]
    pub(crate) thinking: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum RawExpectationItem {
    Explicit(RawExplicitExpectation),
    Generator(RawGeneratorExpectation),
    Include(RawIncludeExpectation),
}

#[derive(Debug, Clone)]
pub(crate) struct RawExplicitExpectation {
    pub(crate) q: String,
    pub(crate) a: String,
    pub(crate) cooldown: Option<String>,
    pub(crate) settings: RawExpectationSettings,
}

#[derive(Debug, Clone)]
pub(crate) struct RawGeneratorExpectation {
    // User-authored generator text that renders an expectation question from a
    // matched file. It is canon data, not a Canon-owned interrogation prompt
    // template; runtime evaluator prompt/instruction templates live under
    // `resources/prompts/`.
    pub(crate) question_template: String,
    pub(crate) path: String,
    pub(crate) a: String,
    pub(crate) cooldown: Option<String>,
    pub(crate) settings: RawExpectationSettings,
}

#[derive(Debug, Clone)]
pub(crate) struct RawIncludeExpectation {
    pub(crate) include: String,
    pub(crate) cooldown: Option<String>,
    pub(crate) settings: RawExpectationSettings,
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
    path: Option<String>,
    #[serde(default)]
    include: Option<String>,
    #[serde(default)]
    cooldown: Option<String>,
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
        if let Some(include) = include {
            return Ok(RawExpectationItem::Include(RawIncludeExpectation {
                include,
                cooldown,
                settings,
            }));
        }
        match (q, q_template, path, a) {
            (_, Some(q_template), Some(path), Some(a)) => {
                Ok(RawExpectationItem::Generator(RawGeneratorExpectation {
                    question_template: q_template,
                    path,
                    a,
                    cooldown,
                    settings,
                }))
            }
            (Some(q), _, _, Some(a)) => Ok(RawExpectationItem::Explicit(RawExplicitExpectation {
                q,
                a,
                cooldown,
                settings,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_shape_wins_over_extra_q_field() {
        let item: RawExpectationItem = serde_saphyr::from_str(
            r#"
q: ignored annotation
path: "specs/*.md"
q_template: "{{content}}"
a: "yes"
"#,
        )
        .expect("parse expectation item");

        match item {
            RawExpectationItem::Generator(item) => {
                assert_eq!(item.path, "specs/*.md");
                assert_eq!(item.question_template, "{{content}}");
                assert_eq!(item.a, "yes");
            }
            RawExpectationItem::Explicit(_) => panic!("generator item parsed as explicit"),
            RawExpectationItem::Include(_) => panic!("generator item parsed as include"),
        }
    }

    #[test]
    fn include_shape_wins_over_extra_question_fields() {
        let item: RawExpectationItem = serde_saphyr::from_str(
            r#"
include: "expects/*.yml"
q: ignored annotation
a: "yes"
"#,
        )
        .expect("parse expectation item");

        match item {
            RawExpectationItem::Include(item) => {
                assert_eq!(item.include, "expects/*.yml");
            }
            RawExpectationItem::Explicit(_) => panic!("include item parsed as explicit"),
            RawExpectationItem::Generator(_) => panic!("include item parsed as generator"),
        }
    }
}
