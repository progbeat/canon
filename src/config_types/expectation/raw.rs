use super::super::{
    deserialize_non_null_configured_value, deserialize_optional_scalar_string,
    deserialize_optional_string, ConfiguredValue, RawExpectationSettings,
};
use super::resolved::{ExpectationTo, InPlaceCompatibility, InPlaceIncompatibleField};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(transparent)]
// [m] A transparent String accepts only the canonical scalar duration form;
// YAML mappings cannot deserialize into this type.
pub(crate) struct CooldownConfig(pub(crate) String);

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum QScopeConfig {
    Mode(String),
    Paths(Vec<String>),
}

#[derive(Debug, Clone)]
pub(crate) enum RawExpectationItem {
    Unresolved(RawExpectationFields),
    Explicit(RawExplicitExpectation),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RawGitBackedExpectationConfig {
    // [m,90] These YAML fields belong to the Git-backed check contract. In
    // particular, Cached Result is defined only for an expectation and Git
    // state, so its optional cooldown is fully supported here. The separate
    // in-place contract later rejects configured fields that require cache,
    // diff, or other Git-backed behavior.
    pub(crate) diff_from: ConfiguredValue<String>,
    pub(crate) target: ConfiguredValue<String>,
    pub(crate) cooldown: ConfiguredValue<CooldownConfig>,
    pub(crate) q_scope: ConfiguredValue<QScopeConfig>,
}

impl RawGitBackedExpectationConfig {
    pub(crate) fn in_place_compatibility(&self) -> InPlaceCompatibility {
        // [90,T5] In-place compatibility depends only on whether a field was
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
            self.q_scope
                .configured
                .then_some(InPlaceIncompatibleField::QScope),
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

impl RawExpectationCommonConfig {
    pub(crate) fn in_place_compatibility(&self) -> InPlaceCompatibility {
        let mut compatibility = self.git_backed.in_place_compatibility();
        if self.settings.ignore.configured {
            compatibility = compatibility.with_incompatible_field(InPlaceIncompatibleField::Ignore);
        }
        compatibility
    }

    pub(crate) fn fill_missing_from(&mut self, defaults: &RawExpectationCommonConfig) {
        if !self.question_context.configured {
            self.question_context = defaults.question_context.clone();
        }
        if !self.git_backed.diff_from.configured {
            self.git_backed.diff_from = defaults.git_backed.diff_from.clone();
        }
        if !self.git_backed.target.configured {
            self.git_backed.target = defaults.git_backed.target.clone();
        }
        if !self.git_backed.cooldown.configured {
            self.git_backed.cooldown = defaults.git_backed.cooldown.clone();
        }
        if !self.git_backed.q_scope.configured {
            self.git_backed.q_scope = defaults.git_backed.q_scope.clone();
        }
        if !self.to.configured {
            self.to = defaults.to.clone();
        }
        if !self.rank.configured {
            self.rank = defaults.rank.clone();
        }
        if !self.settings.models.configured {
            self.settings.models = defaults.settings.models.clone();
        }
        if !self.settings.thinking.configured {
            self.settings.thinking = defaults.settings.thinking.clone();
        }
        if !self.settings.ignore.configured {
            self.settings.ignore = defaults.settings.ignore.clone();
        }
        if !self.settings.plugins.configured {
            self.settings.plugins = defaults.settings.plugins.clone();
        }
    }
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
    #[serde(default, deserialize_with = "deserialize_non_null_configured_value")]
    to: ConfiguredValue<ExpectationTo>,
    #[serde(default)]
    rank: ConfiguredValue<i64>,
    #[serde(default)]
    cooldown: ConfiguredValue<CooldownConfig>,
    #[serde(default)]
    #[serde(rename = "q-scope")]
    q_scope: ConfiguredValue<QScopeConfig>,
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
            q_scope,
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
                    q_scope,
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
    pub(crate) fn common(&self) -> &RawExpectationCommonConfig {
        match self {
            RawExpectationItem::Unresolved(fields) => &fields.common,
            RawExpectationItem::Explicit(expectation) => &expectation.common,
        }
    }

    pub(crate) fn from_resolved_fields(
        fields: RawExpectationFields,
    ) -> Result<RawExpectationItem, &'static str> {
        let RawExpectationFields {
            explicit_q,
            a,
            common,
        } = fields;
        let a = resolve_expected_answer(common.to.resolved_or_implementation_default(), a)?;
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
