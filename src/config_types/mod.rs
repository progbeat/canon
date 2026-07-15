use serde::de::Visitor;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone)]
// Resolved runtime config for check execution. `RawCheckConfig` is the parsed
// check.yml schema with the canon `presets` mapping; expansion resolves preset
// defaults into this type's agent and expectation fields.
pub(crate) struct CheckConfig {
    pub(crate) version: u32,
    pub(crate) agent: AgentConfig,
    pub(crate) expectations: Vec<Expectation>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
// Parsed check.yml schema before preset resolution.
pub(crate) struct RawCheckConfig {
    #[serde(default = "default_config_version")]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) presets: Option<BTreeMap<String, RawPresetConfig>>,
    #[serde(default)]
    pub(crate) agent: Option<RawLegacyAgentConfig>,
    #[serde(deserialize_with = "deserialize_expectation_items")]
    pub(crate) expectations: Vec<RawExpectationItem>,
}

fn default_config_version() -> u32 {
    1
}

fn deserialize_optional_scalar_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct OptionalScalarStringVisitor;

    impl<'de> Visitor<'de> for OptionalScalarStringVisitor {
        type Value = Option<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a scalar value")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
            Ok(Some(value.to_string()))
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
            Ok(Some(value.to_string()))
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
            Ok(Some(value.to_string()))
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
            Ok(Some(value.to_string()))
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
            Ok(Some(value.to_string()))
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
            Ok(Some(value))
        }
    }

    deserializer.deserialize_any(OptionalScalarStringVisitor)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawExpectationEntry {
    Item(Box<RawExpectationItem>),
    Items(Vec<RawExpectationEntry>),
}

fn deserialize_expectation_items<'de, D>(
    deserializer: D,
) -> Result<Vec<RawExpectationItem>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    fn flatten(entry: RawExpectationEntry, output: &mut Vec<RawExpectationItem>) {
        match entry {
            RawExpectationEntry::Item(item) => output.push(*item),
            RawExpectationEntry::Items(items) => {
                for item in items {
                    flatten(item, output);
                }
            }
        }
    }

    let entries = Vec::<RawExpectationEntry>::deserialize(deserializer)?;
    let mut output = Vec::new();
    for entry in entries {
        flatten(entry, &mut output);
    }
    Ok(output)
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
    #[serde(default, deserialize_with = "deserialize_optional_scalar_string")]
    pub(crate) q: Option<String>,
    #[serde(default)]
    pub(crate) q_template: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_scalar_string")]
    pub(crate) a: Option<String>,
    #[serde(default)]
    pub(crate) glob: Option<String>,
    #[serde(default)]
    pub(crate) include: Option<String>,
    #[serde(default)]
    pub(crate) to: Option<ExpectationTo>,
    #[serde(default)]
    pub(crate) rank: Option<i64>,
    #[serde(default)]
    // Human-authored expectation context data inherited by expectation items.
    // Despite the config key name, this is not an implementation-owned
    // evaluator-agent instruction source; only resource templates under
    // `resources/prompts/` decide how to embed it.
    #[serde(rename = "instructions")]
    pub(crate) question_context: Option<String>,
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
    pub(crate) q: Option<String>,
    pub(crate) q_template: Option<String>,
    pub(crate) a: Option<String>,
    pub(crate) glob: Option<String>,
    pub(crate) include: Option<String>,
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
    #[serde(default)]
    pub(crate) to: ExpectationTo,
    pub(crate) q: String,
    pub(crate) a: String,
    #[serde(default)]
    pub(crate) rank: i64,
    // Human-authored expectation context data from check config, like `q` and
    // `a`. Despite the config key name, this is not an implementation-owned
    // evaluator-agent prompt or policy source; only the resource template in
    // `resources/prompts/` decides how to embed it.
    #[serde(rename = "instructions")]
    pub(crate) question_context: String,
    pub(crate) diff_from: String,
    #[serde(default)]
    pub(crate) diff_from_configured: bool,
    #[serde(default)]
    pub(crate) target: Option<ExpectationTarget>,
    #[serde(default)]
    pub(crate) question_answer_only: bool,
    #[serde(default, skip)]
    pub(crate) agent: AgentConfig,
    #[serde(default)]
    pub(crate) cooldown: Option<CooldownConfig>,
}

#[derive(Debug, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ExpectationTo {
    #[default]
    Agent,
    Caller,
    Shell,
}

impl ExpectationTo {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ExpectationTo::Agent => "agent",
            ExpectationTo::Caller => "caller",
            ExpectationTo::Shell => "shell",
        }
    }
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
#[serde(transparent)]
pub(crate) struct CooldownConfig(pub(crate) String);

#[derive(Debug, Clone)]
pub(crate) enum RawExpectationItem {
    Unresolved(RawExpectationFields),
    Explicit(RawExplicitExpectation),
    // The Expectations spec calls both `include` and `glob`/`q_template`/`a`
    // forms generator items. Internally they stay split so config expansion can
    // route include recursion separately from per-file question generation.
    Generator(RawGeneratorExpectation),
    Include(RawIncludeExpectation),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RawExpectationCommonConfig {
    // Raw config data shared by presets, explicit expectations, generated
    // expectations, and includes. It is not an implementation-owned evaluator
    // prompt or policy source.
    pub(crate) question_context: Option<String>,
    pub(crate) diff_from: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) cooldown: Option<CooldownConfig>,
    pub(crate) to: Option<ExpectationTo>,
    pub(crate) rank: Option<i64>,
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
    pub(crate) q: RawGeneratedExpectationQuestion,
    pub(crate) glob: String,
    pub(crate) a: String,
    pub(crate) common: RawExpectationCommonConfig,
}

#[derive(Debug, Clone)]
pub(crate) struct RawGeneratedExpectationQuestion {
    // `q_template` is external configuration data for generated expectation
    // questions. Rendering it produces the generated expectation item's `q`,
    // which has item-level precedence over an explicit-form `q` preset default.
    pub(crate) q_template: String,
}

#[derive(Debug, Clone)]
pub(crate) struct RawIncludeExpectation {
    pub(crate) include: String,
    pub(crate) generated_item_defaults: RawExpectationFields,
}

#[derive(Debug, Clone)]
pub(crate) struct RawExpectationFields {
    // YAML `q` is the explicit-expectation question field. Glob generators do
    // not use this spelling for their item-level q; they render `q_template`.
    pub(crate) explicit_q: Option<String>,
    // In the glob-generator form, `q_template` is the item-level source for the
    // generated expectation's q.
    pub(crate) q_template: Option<String>,
    pub(crate) a: Option<String>,
    pub(crate) glob: Option<String>,
    pub(crate) include: Option<String>,
    pub(crate) common: RawExpectationCommonConfig,
}

#[derive(Clone, Copy)]
pub(crate) enum RawExpectationItemForm {
    Explicit,
    Generator,
    Include,
}

impl RawExpectationItemForm {
    pub(crate) fn from_shape_fields(
        has_q: bool,
        has_q_template: bool,
        has_glob: bool,
        has_include: bool,
    ) -> Option<Self> {
        if has_include {
            return Some(Self::Include);
        }
        if has_q {
            return Some(Self::Explicit);
        }
        if has_glob || has_q_template {
            return Some(Self::Generator);
        }
        None
    }
}

impl RawExpectationFields {
    pub(crate) fn declared_item_form(&self) -> Option<RawExpectationItemForm> {
        RawExpectationItemForm::from_shape_fields(
            self.explicit_q.is_some(),
            self.q_template.is_some(),
            self.glob.is_some(),
            self.include.is_some(),
        )
    }
}

#[derive(Debug, Deserialize)]
// Expectation items intentionally omit `deny_unknown_fields`: the expectations
// spec allows extra fields so external IDs or annotations can stay in check files
// without affecting canon's explicit/generator/include expansion.
struct RawExpectationFieldValues {
    #[serde(default, deserialize_with = "deserialize_optional_scalar_string")]
    q: Option<String>,
    #[serde(default)]
    q_template: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_scalar_string")]
    a: Option<String>,
    #[serde(default)]
    // Human-authored canon data for one expectation item, not a prompt
    // template defined by this config parser.
    #[serde(rename = "instructions")]
    question_context: Option<String>,
    #[serde(default)]
    #[serde(rename = "diff-from")]
    diff_from: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    glob: Option<String>,
    #[serde(default)]
    include: Option<String>,
    #[serde(default)]
    to: Option<ExpectationTo>,
    #[serde(default)]
    rank: Option<i64>,
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

impl From<RawExpectationFieldValues> for RawExpectationFields {
    fn from(fields: RawExpectationFieldValues) -> RawExpectationFields {
        let RawExpectationFieldValues {
            q,
            q_template,
            a,
            question_context,
            diff_from,
            target,
            glob,
            include,
            to,
            rank,
            cooldown,
            preset,
            models,
            thinking,
            ignore,
            plugins,
        } = fields;
        RawExpectationFields {
            explicit_q: q,
            q_template,
            a,
            glob,
            include,
            common: RawExpectationCommonConfig {
                question_context,
                diff_from,
                target,
                cooldown,
                to,
                rank,
                settings: RawExpectationSettings {
                    preset,
                    models,
                    thinking,
                    ignore,
                    plugins,
                },
            },
        }
    }
}

impl<'de> Deserialize<'de> for RawExpectationItem {
    fn deserialize<D>(deserializer: D) -> Result<RawExpectationItem, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let fields = RawExpectationFieldValues::deserialize(deserializer)?;
        Ok(RawExpectationItem::Unresolved(fields.into()))
    }
}

impl RawExpectationItem {
    pub(crate) fn include_from_fields(
        fields: RawExpectationFields,
    ) -> Result<RawExpectationItem, &'static str> {
        let RawExpectationFields {
            explicit_q,
            q_template,
            a,
            glob,
            include,
            common,
        } = fields;
        match include {
            Some(include) => Ok(RawExpectationItem::Include(RawIncludeExpectation {
                include,
                generated_item_defaults: RawExpectationFields {
                    explicit_q,
                    q_template,
                    a,
                    glob,
                    include: None,
                    common,
                },
            })),
            None => Err("missing required field after default resolution: include"),
        }
    }

    pub(crate) fn generator_from_fields(
        fields: RawExpectationFields,
    ) -> Result<RawExpectationItem, &'static str> {
        let RawExpectationFields {
            q_template,
            a,
            glob,
            common,
            ..
        } = fields;
        let a = resolve_expected_answer(common.to.unwrap_or_default(), a)?;
        match (q_template, glob) {
            (Some(q_template), Some(glob)) => {
                Ok(RawExpectationItem::Generator(RawGeneratorExpectation {
                    q: RawGeneratedExpectationQuestion { q_template },
                    glob,
                    a,
                    common,
                }))
            }
            (Some(_), None) => Err("missing required field after default resolution: glob"),
            (None, Some(_)) => Err("missing required field after default resolution: q_template"),
            _ => Err("invalid expectation item"),
        }
    }

    pub(crate) fn explicit_from_fields(
        fields: RawExpectationFields,
    ) -> Result<RawExpectationItem, &'static str> {
        let RawExpectationFields {
            explicit_q,
            a,
            common,
            ..
        } = fields;
        let a = resolve_expected_answer(common.to.unwrap_or_default(), a)?;
        match explicit_q {
            Some(q) => Ok(RawExpectationItem::Explicit(RawExplicitExpectation {
                q,
                a,
                common,
            })),
            _ => Err("invalid expectation item"),
        }
    }

    pub(crate) fn from_resolved_fields(
        fields: RawExpectationFields,
    ) -> Result<RawExpectationItem, &'static str> {
        let RawExpectationFields {
            explicit_q,
            q_template,
            a,
            glob,
            include,
            common,
        } = fields;
        // Field resolution has already applied item values, selected preset
        // values, and real implementation defaults. Required shape fields have
        // no synthetic defaults: inventing an `include`, `q`+`a`, or
        // `glob`+`q_template`+`a` would change the expectation form instead of
        // resolving it, so absence after resolution is a config error.
        if let Some(include) = include {
            return Ok(RawExpectationItem::Include(RawIncludeExpectation {
                include,
                generated_item_defaults: RawExpectationFields {
                    explicit_q,
                    q_template,
                    a,
                    glob,
                    include: None,
                    common,
                },
            }));
        }
        let a = resolve_expected_answer(common.to.unwrap_or_default(), a)?;
        match (explicit_q, q_template, glob) {
            (_, Some(q_template), Some(glob)) => {
                Ok(RawExpectationItem::Generator(RawGeneratorExpectation {
                    q: RawGeneratedExpectationQuestion { q_template },
                    glob,
                    a,
                    common,
                }))
            }
            (Some(q), _, _) => Ok(RawExpectationItem::Explicit(RawExplicitExpectation {
                q,
                a,
                common,
            })),
            fields => match fields {
                (None, Some(_), None) => {
                    Err("missing required field after default resolution: glob")
                }
                (None, None, Some(_)) => {
                    Err("missing required field after default resolution: q_template")
                }
                (None, None, None) => Err(
                    "missing required field after default resolution: q, q_template, or include",
                ),
                _ => Err("invalid expectation item"),
            },
        }
    }
}

fn resolve_expected_answer(
    to: ExpectationTo,
    answer: Option<String>,
) -> Result<String, &'static str> {
    match (to, answer) {
        (ExpectationTo::Shell, None) => Ok("0".to_string()),
        (ExpectationTo::Shell, Some(answer)) if answer.is_empty() => Ok("0".to_string()),
        (_, Some(answer)) => Ok(answer),
        (_, None) => Err("missing required field after default resolution: a"),
    }
}
