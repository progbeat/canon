mod agent;
mod expectation;
mod preset;

use serde::de::Visitor;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;

pub(crate) use agent::{AgentConfig, RawExpectationSettings, RawLegacyAgentConfig};
pub(crate) use expectation::{
    Cooldown, CooldownConfig, Expectation, ExpectationTarget, ExpectationTo,
    InPlaceIncompatibleField, QScope, QScopeConfig, RawExpectationCommonConfig,
    RawExpectationFields, RawExpectationItem, RawGitBackedExpectationConfig,
    AGAINST_TREE_DIFF_FROM, DEFAULT_DIFF_FROM,
};
pub(crate) use preset::{preset_names_in_precedence_order, RawPresetConfig, ResolvedPresetConfig};

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

pub(super) fn deserialize_optional_scalar_string<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // [MH] YAML expected-answer sources may be non-string scalars. This
    // boundary only stringifies their representation; it does not make every
    // resulting string an admissible answer. Resolved config validation later
    // applies the selected evaluator schema's answer pattern independently.
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

pub(super) fn deserialize_optional_string<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
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

    pub(crate) fn resolved_or_implementation_default(&self) -> T
    where
        T: Clone + Default,
    {
        // [1H,MH] The `to` field's non-null deserializer ensures None means
        // the field was omitted through every precedence layer.
        self.value.clone().unwrap_or_default()
    }
}

pub(super) fn deserialize_non_null_configured_value<'de, D, T>(
    deserializer: D,
) -> Result<ConfiguredValue<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(ConfiguredValue::some)
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
        // [T5] The wrapper itself, rather than its optional value, receives the
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
