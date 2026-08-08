//! Context-related fields in the Codex runtime configuration.

use super::super::codec::{ConfigEntry, ConfigEntryValue};
use super::super::EVALUATOR_DISABLED_FEATURES;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize)]
pub(super) struct EvaluatorContextIsolation {
    include_environment_context: bool,
    include_permissions_context: bool,
    include_apps_context: bool,
    features: BTreeMap<String, bool>,
    project_doc_max_bytes: u64,
}

impl EvaluatorContextIsolation {
    pub(super) fn disabled() -> EvaluatorContextIsolation {
        EvaluatorContextIsolation {
            include_environment_context: false,
            include_permissions_context: false,
            include_apps_context: false,
            features: evaluator_context_isolation_features()
                .map(|feature| (feature.to_string(), false))
                .collect(),
            project_doc_max_bytes: 0,
        }
    }

    pub(super) fn push_config_entries(&self, entries: &mut Vec<ConfigEntry>) {
        entries.push(ConfigEntry::bool(
            ["include_environment_context"],
            self.include_environment_context,
        ));
        entries.push(ConfigEntry::bool(
            ["include_permissions_instructions"],
            self.include_permissions_context,
        ));
        entries.push(ConfigEntry::bool(
            ["include_apps_instructions"],
            self.include_apps_context,
        ));
        for (feature, enabled) in &self.features {
            entries.push(ConfigEntry {
                path: vec!["features".to_string(), feature.clone()],
                value: ConfigEntryValue::Bool(*enabled),
            });
        }
        entries.push(ConfigEntry::u64(
            ["project_doc_max_bytes"],
            self.project_doc_max_bytes,
        ));
    }
}

fn evaluator_context_isolation_features() -> impl Iterator<Item = &'static str> {
    EVALUATOR_DISABLED_FEATURES.iter().copied()
}
