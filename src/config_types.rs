use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckConfig {
    pub(crate) version: u32,
    pub(crate) agent: AgentConfig,
    pub(crate) expectations: Vec<Expectation>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawCheckConfig {
    pub(crate) version: u32,
    pub(crate) agent: AgentConfig,
    pub(crate) expectations: Vec<RawExpectationItem>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentConfig {
    #[serde(default)]
    pub(crate) model: ModelConfig,
    #[serde(default = "default_thinking")]
    pub(crate) thinking: String,
    #[serde(default)]
    pub(crate) instructions: Option<String>,
    pub(crate) ignore: Vec<String>,
    pub(crate) plugins: Vec<String>,
}

impl AgentConfig {
    pub(crate) fn custom_instructions(&self) -> &str {
        self.instructions.as_deref().unwrap_or("")
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelConfig {
    #[serde(default)]
    pub(crate) primary: Option<String>,
    #[serde(default)]
    pub(crate) fallbacks: Vec<String>,
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
    pub(crate) thinking: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RawGeneratorExpectation {
    pub(crate) q_template: String,
    pub(crate) path: String,
    pub(crate) a: String,
    pub(crate) cooldown: Option<String>,
    pub(crate) thinking: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RawIncludeExpectation {
    pub(crate) include: String,
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
    thinking: Option<String>,
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
            thinking,
        } = fields;
        match (q, q_template, path, a) {
            (Some(q), _, _, Some(a)) => Ok(RawExpectationItem::Explicit(RawExplicitExpectation {
                q,
                a,
                cooldown,
                thinking,
            })),
            (None, Some(q_template), Some(path), Some(a)) => {
                Ok(RawExpectationItem::Generator(RawGeneratorExpectation {
                    q_template,
                    path,
                    a,
                    cooldown,
                    thinking,
                }))
            }
            fields => {
                if let Some(include) = include {
                    return Ok(RawExpectationItem::Include(RawIncludeExpectation {
                        include,
                    }));
                }
                match fields {
                    (Some(_), _, _, None) => Err("must contain a"),
                    (None, Some(_), None, _) => Err("generator must contain path"),
                    (None, Some(_), Some(_), None) => Err("must contain a"),
                    (None, None, Some(_), _) => Err("generator must contain q_template"),
                    (None, None, None, Some(_)) => Err("must contain q or q_template"),
                    (None, None, None, None) => Err("must contain q, q_template, or include"),
                    _ => Err("invalid expectation item"),
                }
            }
        }
    }
}
