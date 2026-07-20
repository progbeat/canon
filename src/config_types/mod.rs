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
    #[serde(alias = "xpecs", deserialize_with = "deserialize_expectation_items")]
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
            Ok(Some("null".to_string()))
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(Some("null".to_string()))
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

fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct OptionalStringVisitor;

    impl<'de> Visitor<'de> for OptionalStringVisitor {
        type Value = Option<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a string")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
            Ok(Some(value.to_string()))
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
            Ok(Some(value))
        }
    }

    deserializer.deserialize_any(OptionalStringVisitor)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfiguredValue<T> {
    pub(crate) value: Option<T>,
    pub(crate) configured: bool,
}

impl<T> ConfiguredValue<T> {
    pub(crate) fn from_option(value: Option<T>) -> ConfiguredValue<T> {
        ConfiguredValue {
            configured: value.is_some(),
            value,
        }
    }

    pub(crate) fn some(value: T) -> ConfiguredValue<T> {
        ConfiguredValue {
            value: Some(value),
            configured: true,
        }
    }
}

impl<T> Default for ConfiguredValue<T> {
    fn default() -> ConfiguredValue<T> {
        ConfiguredValue {
            value: None,
            configured: false,
        }
    }
}

impl<'de, T> Deserialize<'de> for ConfiguredValue<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<ConfiguredValue<T>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // [T] The wrapper itself, rather than its optional value, receives the
        // YAML field. That keeps explicit null distinct from an omitted field.
        Ok(ConfiguredValue {
            value: Option::<T>::deserialize(deserializer)?,
            configured: true,
        })
    }
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
pub(crate) struct RawPresetConfig {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub(crate) q: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_scalar_string")]
    pub(crate) a: Option<String>,
    #[serde(default)]
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

#[derive(Debug, Clone, Default)]
pub(crate) struct ResolvedPresetConfig {
    pub(crate) q: Option<String>,
    pub(crate) a: Option<String>,
    pub(crate) common: RawExpectationCommonConfig,
}

impl ResolvedPresetConfig {
    pub(crate) fn agent_config(&self) -> AgentConfig {
        let mut agent = AgentConfig::implementation_default();
        let settings = &self.common.settings;
        if let Some(models) = &settings.models.value {
            agent.models = models.clone();
        }
        if let Some(thinking) = &settings.thinking.value {
            agent.thinking = thinking.clone();
        }
        agent.ignore = settings.ignore.value.clone();
        agent.ignore_configured = settings.ignore.configured;
        if let Some(plugins) = &settings.plugins.value {
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

pub(crate) fn default_thinking() -> String {
    "low".to_string()
}

pub(crate) const DEFAULT_DIFF_FROM: &str = ":checkpoint";
pub(crate) const AGAINST_TREE_DIFF_FROM: &str = ":against-tree";

#[derive(Debug, Clone)]
pub(crate) struct Expectation {
    pub(crate) to: ExpectationTo,
    pub(crate) q: String,
    pub(crate) a: String,
    // [IJ] Canon check orders ascending by rank; omitted config resolves to 0.
    pub(crate) rank: i64,
    // Human-authored expectation context data from check config, like `q` and
    // `a`. Despite the config key name, this is not an implementation-owned
    // evaluator-agent prompt or policy source; only the resource template in
    // `resources/prompts/` decides how to embed it.
    pub(crate) question_context: String,
    pub(crate) diff_from: String,
    pub(crate) target: Option<ExpectationTarget>,
    pub(crate) question_answer_only: bool,
    pub(crate) agent: AgentConfig,
    pub(crate) cooldown: Option<Cooldown>,
    // Raw expansion classifies mode compatibility once; later validation
    // consumes this typed domain result without recovering it from values or
    // field-name strings.
    pub(crate) in_place_compatibility: InPlaceCompatibility,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum InPlaceCompatibility {
    #[default]
    Compatible,
    Incompatible(Vec<InPlaceIncompatibleField>),
}

impl InPlaceCompatibility {
    pub(crate) fn with_incompatible_field(self, field: InPlaceIncompatibleField) -> Self {
        match self {
            InPlaceCompatibility::Compatible => InPlaceCompatibility::Incompatible(vec![field]),
            InPlaceCompatibility::Incompatible(mut fields) => {
                fields.push(field);
                InPlaceCompatibility::Incompatible(fields)
            }
        }
    }

    pub(crate) fn incompatible_fields(&self) -> &[InPlaceIncompatibleField] {
        match self {
            InPlaceCompatibility::Compatible => &[],
            InPlaceCompatibility::Incompatible(fields) => fields,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InPlaceIncompatibleField {
    DiffFrom,
    Target,
    Cooldown,
    Ignore,
}

impl InPlaceIncompatibleField {
    pub(crate) fn config_name(self) -> &'static str {
        match self {
            InPlaceIncompatibleField::DiffFrom => "diff-from",
            InPlaceIncompatibleField::Target => "target",
            InPlaceIncompatibleField::Cooldown => "cooldown",
            InPlaceIncompatibleField::Ignore => "ignore",
        }
    }
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
// [uf] A transparent String accepts only the canonical scalar duration form;
// YAML mappings cannot deserialize into this type.
pub(crate) struct CooldownConfig(pub(crate) String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cooldown {
    pub(crate) seconds: u64,
}

#[derive(Debug, Clone)]
pub(crate) enum RawExpectationItem {
    Unresolved(RawExpectationFields),
    Explicit(RawExplicitExpectation),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RawGitBackedExpectationConfig {
    // [uf,I4] These YAML fields belong to the Git-backed check contract. In
    // particular, Cached Result is defined only for an expectation and Git
    // state, so its optional cooldown is fully supported here. The separate
    // in-place contract later rejects configured fields that require cache,
    // diff, or other Git-backed behavior.
    pub(crate) diff_from: ConfiguredValue<String>,
    pub(crate) target: ConfiguredValue<String>,
    pub(crate) cooldown: ConfiguredValue<CooldownConfig>,
}

impl RawGitBackedExpectationConfig {
    pub(crate) fn in_place_compatibility(&self) -> InPlaceCompatibility {
        // [cg,eS,T] In-place compatibility depends only on whether a field was
        // configured, including an explicit YAML null. Target values share the
        // same prohibition; only prompt rendering later distinguishes them.
        [
            self.diff_from
                .configured
                .then_some(InPlaceIncompatibleField::DiffFrom),
            self.target
                .configured
                .then_some(InPlaceIncompatibleField::Target),
            self.cooldown
                .configured
                .then_some(InPlaceIncompatibleField::Cooldown),
        ]
        .into_iter()
        .flatten()
        .fold(InPlaceCompatibility::Compatible, |compatibility, field| {
            compatibility.with_incompatible_field(field)
        })
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RawExpectationCommonConfig {
    // Raw config data shared by presets and expectation items. It is not an
    // implementation-owned evaluator prompt or policy source.
    pub(crate) question_context: ConfiguredValue<String>,
    pub(crate) git_backed: RawGitBackedExpectationConfig,
    pub(crate) to: ConfiguredValue<ExpectationTo>,
    pub(crate) rank: ConfiguredValue<i64>,
    pub(crate) settings: RawExpectationSettings,
}

#[derive(Debug, Clone)]
pub(crate) struct RawExplicitExpectation {
    pub(crate) q: String,
    pub(crate) a: String,
    pub(crate) common: RawExpectationCommonConfig,
}

#[derive(Debug, Clone)]
pub(crate) struct RawExpectationFields {
    pub(crate) explicit_q: Option<String>,
    pub(crate) a: Option<String>,
    pub(crate) common: RawExpectationCommonConfig,
}

#[derive(Debug, Deserialize)]
// Expectation items intentionally omit `deny_unknown_fields`: the xpecs spec
// allows extra fields so external IDs or annotations can stay in check files.
struct RawExpectationFieldValues {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    q: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_scalar_string")]
    a: Option<String>,
    #[serde(default)]
    // Human-authored canon data for one expectation item, not a prompt
    // template defined by this config parser.
    #[serde(rename = "instructions")]
    question_context: ConfiguredValue<String>,
    #[serde(default)]
    #[serde(rename = "diff-from")]
    diff_from: ConfiguredValue<String>,
    #[serde(default)]
    target: ConfiguredValue<String>,
    #[serde(default)]
    to: ConfiguredValue<ExpectationTo>,
    #[serde(default)]
    rank: ConfiguredValue<i64>,
    #[serde(default)]
    cooldown: ConfiguredValue<CooldownConfig>,
    #[serde(default)]
    preset: Option<String>,
    #[serde(default)]
    models: ConfiguredValue<Vec<String>>,
    #[serde(default)]
    thinking: ConfiguredValue<String>,
    #[serde(default)]
    ignore: ConfiguredValue<Vec<String>>,
    #[serde(default)]
    plugins: ConfiguredValue<Vec<String>>,
}

impl From<RawExpectationFieldValues> for RawExpectationFields {
    fn from(fields: RawExpectationFieldValues) -> RawExpectationFields {
        let RawExpectationFieldValues {
            q,
            a,
            question_context,
            diff_from,
            target,
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
            a,
            common: RawExpectationCommonConfig {
                question_context,
                git_backed: RawGitBackedExpectationConfig {
                    diff_from,
                    target,
                    cooldown,
                },
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
    pub(crate) fn from_resolved_fields(
        fields: RawExpectationFields,
    ) -> Result<RawExpectationItem, &'static str> {
        let RawExpectationFields {
            explicit_q,
            a,
            common,
        } = fields;
        let a = resolve_expected_answer(common.to.value.unwrap_or_default(), a)?;
        match explicit_q {
            Some(q) => Ok(RawExpectationItem::Explicit(RawExplicitExpectation {
                q,
                a,
                common,
            })),
            None => Err("missing required field after default resolution: q"),
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
