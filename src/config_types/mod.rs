use serde::de::{self, Visitor};
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
    // Resolved top-level check hooks from the parsed `hooks` mapping.
    pub(crate) hooks: CheckHooksConfig,
    pub(crate) expectations: Vec<Expectation>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
// Parsed check.yml schema before preset resolution.
pub(crate) struct RawCheckConfig {
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) presets: Option<BTreeMap<String, RawPresetConfig>>,
    #[serde(default)]
    // Optional top-level `hooks` mapping from check.yml. Expansion resolves
    // this raw schema into `CheckConfig.hooks`.
    pub(crate) hooks: Option<RawCheckHooksConfig>,
    #[serde(default)]
    pub(crate) agent: Option<RawLegacyAgentConfig>,
    pub(crate) expectations: Vec<RawExpectationItem>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CheckHooksConfig {
    pub(crate) on_start: Vec<CheckHookConfig>,
    pub(crate) on_pass: Vec<CheckHookConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckHookConfig {
    pub(crate) print: Option<String>,
    pub(crate) input: Option<String>,
    pub(crate) exec: Option<Vec<String>>,
    pub(crate) cases: BTreeMap<String, CheckHookCaseOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CheckHookCaseOutcome {
    Continue,
    Block { repair_instruction: String },
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawCheckHooksConfig {
    #[serde(default)]
    #[serde(rename = "on-start")]
    pub(crate) on_start: Option<RawCheckHookConfig>,
    #[serde(default)]
    #[serde(rename = "on-pass")]
    pub(crate) on_pass: Option<RawCheckHookConfig>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub(crate) enum RawCheckHookConfig {
    Shorthand(String),
    Mapping(RawCheckHookMappingConfig),
    List(Vec<RawCheckHookConfig>),
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawCheckHookMappingConfig {
    #[serde(default)]
    pub(crate) print: Option<String>,
    #[serde(default)]
    pub(crate) input: Option<String>,
    #[serde(default)]
    pub(crate) exec: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) cases: Option<BTreeMap<RawCheckHookCaseKey, RawCheckHookCaseOutcome>>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub(crate) enum RawCheckHookCaseOutcome {
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "block")]
    Block(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RawCheckHookCaseKey(String);

impl RawCheckHookCaseKey {
    fn into_string(self) -> String {
        self.0
    }
}

pub(crate) const DEFAULT_CHECK_HOOK_REPAIR_INSTRUCTION: &str =
    "▷ Fix the blocker and run `canon check` again!";

impl RawCheckHooksConfig {
    pub(crate) fn resolve(self) -> Result<CheckHooksConfig, String> {
        Ok(CheckHooksConfig {
            on_start: self
                .on_start
                .map(|hooks| hooks.resolve("hooks.on-start"))
                .transpose()?
                .unwrap_or_default(),
            on_pass: self
                .on_pass
                .map(|hooks| hooks.resolve("hooks.on-pass"))
                .transpose()?
                .unwrap_or_default(),
        })
    }
}

impl RawCheckHookConfig {
    pub(crate) fn resolve(self, label: &str) -> Result<Vec<CheckHookConfig>, String> {
        match self {
            RawCheckHookConfig::List(hooks) => hooks
                .into_iter()
                .enumerate()
                .map(|(index, hook)| hook.resolve(&format!("{}[{}]", label, index)))
                .collect::<Result<Vec<_>, _>>()
                .map(|hooks| hooks.into_iter().flatten().collect()),
            hook => hook.resolve_one(label).map(|hook| vec![hook]),
        }
    }

    fn resolve_one(self, label: &str) -> Result<CheckHookConfig, String> {
        match self {
            RawCheckHookConfig::Shorthand(print) => Ok(CheckHookConfig {
                print: Some(print),
                input: None,
                exec: None,
                cases: BTreeMap::new(),
            }),
            RawCheckHookConfig::Mapping(mapping) => resolve_hook_mapping(label, mapping),
            RawCheckHookConfig::List(_) => unreachable!("hook list is resolved before hook item"),
        }
    }
}

fn resolve_hook_mapping(
    label: &str,
    mapping: RawCheckHookMappingConfig,
) -> Result<CheckHookConfig, String> {
    let has_print = mapping.print.is_some();
    let has_input = mapping.input.is_some();
    let has_exec = mapping.exec.is_some();
    let has_cases = mapping.cases.is_some();
    if !has_print && !has_input && !has_exec {
        return Err(format!("{} must contain print, input, or exec", label));
    }
    if has_input && has_exec {
        return Err(format!("{} input and exec are mutually exclusive", label));
    }
    let cases = mapping
        .cases
        .unwrap_or_default()
        .into_iter()
        .map(|(key, outcome)| (key.into_string(), outcome.resolve()))
        .collect::<BTreeMap<_, _>>();
    if has_cases && !has_input && !has_exec {
        return Err(format!("{} cases requires input or exec", label));
    }
    if (has_input || has_exec) && cases.is_empty() {
        return Err(format!("{} input or exec requires cases", label));
    }
    if mapping.exec.as_ref().is_some_and(Vec::is_empty) {
        return Err(format!("{} exec must not be empty", label));
    }
    Ok(CheckHookConfig {
        print: mapping.print,
        input: mapping.input,
        exec: mapping.exec,
        cases,
    })
}

impl RawCheckHookCaseOutcome {
    fn resolve(self) -> CheckHookCaseOutcome {
        match self {
            RawCheckHookCaseOutcome::Ok => CheckHookCaseOutcome::Continue,
            RawCheckHookCaseOutcome::Block(repair_instruction) => {
                CheckHookCaseOutcome::Block { repair_instruction }
            }
        }
    }
}

impl<'de> Deserialize<'de> for RawCheckHookCaseKey {
    fn deserialize<D>(deserializer: D) -> Result<RawCheckHookCaseKey, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(RawCheckHookCaseKeyVisitor)
    }
}

struct RawCheckHookCaseKeyVisitor;

impl Visitor<'_> for RawCheckHookCaseKeyVisitor {
    type Value = RawCheckHookCaseKey;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a YAML scalar hook case key")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(RawCheckHookCaseKey(value.to_string()))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(RawCheckHookCaseKey(value.to_string()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(RawCheckHookCaseKey(value.to_string()))
    }

    fn visit_i128<E>(self, value: i128) -> Result<Self::Value, E> {
        Ok(RawCheckHookCaseKey(value.to_string()))
    }

    fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E> {
        Ok(RawCheckHookCaseKey(value.to_string()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
        Ok(RawCheckHookCaseKey(value.to_string()))
    }

    fn visit_char<E>(self, value: char) -> Result<Self::Value, E> {
        Ok(RawCheckHookCaseKey(value.to_string()))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RawCheckHookCaseKey(value.to_string()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(RawCheckHookCaseKey(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Err(E::invalid_type(de::Unexpected::Unit, &self))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Err(E::invalid_type(de::Unexpected::Unit, &self))
    }
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
    pub(crate) q: Option<String>,
    #[serde(default)]
    pub(crate) q_template: Option<String>,
    #[serde(default)]
    pub(crate) a: Option<String>,
    #[serde(default)]
    pub(crate) path: Option<String>,
    #[serde(default)]
    pub(crate) include: Option<String>,
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
    pub(crate) path: Option<String>,
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
    pub(crate) q: String,
    pub(crate) a: String,
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
    Unresolved(RawExpectationFields),
    Explicit(RawExplicitExpectation),
    // The Expectations spec calls both `include` and `path`/`q_template`/`a`
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
    pub(crate) path: String,
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
    // YAML `q` is the explicit-expectation question field. Path generators do
    // not use this spelling for their item-level q; they render `q_template`.
    pub(crate) explicit_q: Option<String>,
    // In the path-generator form, `q_template` is the item-level source for the
    // generated expectation's q.
    pub(crate) q_template: Option<String>,
    pub(crate) a: Option<String>,
    pub(crate) path: Option<String>,
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
        has_path: bool,
        has_include: bool,
    ) -> Option<Self> {
        if has_include {
            return Some(Self::Include);
        }
        if has_path || has_q_template {
            return Some(Self::Generator);
        }
        if has_q {
            return Some(Self::Explicit);
        }
        None
    }
}

impl RawExpectationFields {
    pub(crate) fn declared_item_form(&self) -> Option<RawExpectationItemForm> {
        RawExpectationItemForm::from_shape_fields(
            self.explicit_q.is_some(),
            self.q_template.is_some(),
            self.path.is_some(),
            self.include.is_some(),
        )
    }
}

#[derive(Debug, Deserialize)]
// Expectation items intentionally omit `deny_unknown_fields`: the expectations
// spec allows extra fields so external IDs or annotations can stay in check files
// without affecting canon's explicit/generator/include expansion.
struct RawExpectationFieldValues {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    q_template: Option<String>,
    #[serde(default)]
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

impl From<RawExpectationFieldValues> for RawExpectationFields {
    fn from(fields: RawExpectationFieldValues) -> RawExpectationFields {
        let RawExpectationFieldValues {
            q,
            q_template,
            a,
            question_context,
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
        RawExpectationFields {
            explicit_q: q,
            q_template,
            a,
            path,
            include,
            common: RawExpectationCommonConfig {
                question_context,
                diff_from,
                target,
                cooldown,
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
    pub(crate) fn from_fields_with_resolved_form(
        fields: RawExpectationFields,
        resolved_form: Option<RawExpectationItemForm>,
    ) -> Result<RawExpectationItem, &'static str> {
        let RawExpectationFields {
            explicit_q,
            q_template,
            a,
            path,
            include,
            common,
        } = fields;
        // Field resolution has already applied item values, selected preset
        // values, and real implementation defaults. Required shape fields have
        // no synthetic defaults: inventing an `include`, `q`+`a`, or
        // `path`+`q_template`+`a` would change the expectation form instead of
        // resolving it, so absence after resolution is a config error.
        match resolved_form {
            Some(RawExpectationItemForm::Include) => {
                return match include {
                    Some(include) => Ok(RawExpectationItem::Include(RawIncludeExpectation {
                        include,
                        generated_item_defaults: RawExpectationFields {
                            explicit_q,
                            q_template,
                            a,
                            path,
                            include: None,
                            common,
                        },
                    })),
                    None => Err("missing required field after default resolution: include"),
                };
            }
            Some(RawExpectationItemForm::Generator) => {
                return match (q_template, path, a) {
                    (Some(q_template), Some(path), Some(a)) => {
                        Ok(RawExpectationItem::Generator(RawGeneratorExpectation {
                            q: RawGeneratedExpectationQuestion { q_template },
                            path,
                            a,
                            common,
                        }))
                    }
                    (Some(_), None, _) => {
                        Err("missing required field after default resolution: path")
                    }
                    (Some(_), Some(_), None) => {
                        Err("missing required field after default resolution: a")
                    }
                    (None, Some(_), _) => {
                        Err("missing required field after default resolution: q_template")
                    }
                    _ => Err("invalid expectation item"),
                };
            }
            Some(RawExpectationItemForm::Explicit) => {
                return match (explicit_q, a) {
                    (Some(q), Some(a)) => {
                        Ok(RawExpectationItem::Explicit(RawExplicitExpectation {
                            q,
                            a,
                            common,
                        }))
                    }
                    (Some(_), None) => Err("missing required field after default resolution: a"),
                    _ => Err("invalid expectation item"),
                };
            }
            None => {}
        }
        if let Some(include) = include {
            return Ok(RawExpectationItem::Include(RawIncludeExpectation {
                include,
                generated_item_defaults: RawExpectationFields {
                    explicit_q,
                    q_template,
                    a,
                    path,
                    include: None,
                    common,
                },
            }));
        }
        match (explicit_q, q_template, path, a) {
            (_, Some(q_template), Some(path), Some(a)) => {
                Ok(RawExpectationItem::Generator(RawGeneratorExpectation {
                    q: RawGeneratedExpectationQuestion { q_template },
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
                (Some(_), _, _, None) => Err("missing required field after default resolution: a"),
                (None, Some(_), None, _) => {
                    Err("missing required field after default resolution: path")
                }
                (None, Some(_), Some(_), None) => {
                    Err("missing required field after default resolution: a")
                }
                (None, None, Some(_), _) => {
                    Err("missing required field after default resolution: q_template")
                }
                (None, None, None, Some(_)) => {
                    Err("missing required field after default resolution: q or q_template")
                }
                (None, None, None, None) => Err(
                    "missing required field after default resolution: q, q_template, or include",
                ),
                _ => Err("invalid expectation item"),
            },
        }
    }
}
