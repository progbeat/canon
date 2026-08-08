use super::super::AgentConfig;
use serde::Deserialize;

pub(crate) const DEFAULT_DIFF_FROM: &str = ":checkpoint";
pub(crate) const AGAINST_TREE_DIFF_FROM: &str = ":against-tree";

#[derive(Debug, Clone)]
pub(crate) struct Expectation {
    pub(crate) to: ExpectationTo,
    pub(crate) q: String,
    pub(crate) a: String,
    // [H9] Canon check orders ascending by rank; omitted config resolves to 0.
    pub(crate) rank: i64,
    // Human-authored expectation context data from check config, like `q` and
    // `a`. Despite the config key name, this is not an implementation-owned
    // evaluator-agent prompt or policy source; only the resource template in
    // `resources/prompts/` decides how to embed it.
    pub(crate) question_context: String,
    pub(crate) diff_from: String,
    pub(crate) target: Option<ExpectationTarget>,
    pub(crate) agent: AgentConfig,
    pub(crate) cooldown: Option<Cooldown>,
    pub(crate) q_scope: QScope,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum InPlaceIncompatibleField {
    DiffFrom,
    Target,
    Cooldown,
    QScope,
    Ignore,
}

impl InPlaceIncompatibleField {
    pub(crate) fn config_name(self) -> &'static str {
        match self {
            InPlaceIncompatibleField::DiffFrom => "diff-from",
            InPlaceIncompatibleField::Target => "target",
            InPlaceIncompatibleField::Cooldown => "cooldown",
            InPlaceIncompatibleField::QScope => "q-scope",
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
    // [hj] `target: diff` changes the subject of evaluation, not the meaning
    // of the question's criterion: only visible files affected by the resolved
    // diff can supply a violation, while other visible files supply context.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cooldown {
    pub(crate) seconds: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum QScope {
    #[default]
    Auto,
    Paths(Vec<String>),
}

impl QScope {
    pub(crate) fn is_auto(&self) -> bool {
        matches!(self, QScope::Auto)
    }

    pub(crate) fn paths(&self) -> Option<&[String]> {
        match self {
            QScope::Auto => None,
            QScope::Paths(paths) => Some(paths),
        }
    }
}
