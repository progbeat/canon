use crate::check::config::expansion::CheckConfigSource;
use crate::repo_inspection::RepoInspectionCache;
use minijinja::value::Value;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum BindingChoices {
    Many(Vec<Value>),
    One(Value),
}

#[derive(Clone)]
pub(super) struct ForeachBindings(Vec<(String, BindingChoices)>);

impl<'de> Deserialize<'de> for ForeachBindings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BindingsVisitor;

        impl<'de> Visitor<'de> for BindingsVisitor {
            type Value = ForeachBindings;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a mapping of !foreach variable names to choices")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut entries = Vec::new();
                let mut names = BTreeSet::new();
                while let Some((name, choices)) = map.next_entry::<String, BindingChoices>()? {
                    if !names.insert(name.clone()) {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate !foreach variable: {name}"
                        )));
                    }
                    entries.push((name, choices));
                }
                Ok(ForeachBindings(entries))
            }
        }

        deserializer.deserialize_map(BindingsVisitor)
    }
}

pub(super) struct ForeachBinding {
    pub(super) name: String,
    pub(super) choices: Vec<Value>,
}

pub(super) fn resolve_foreach_bindings(
    bindings: ForeachBindings,
    inherited: Option<&BTreeMap<String, Value>>,
    root: &Path,
    config_path: &Path,
    source: &CheckConfigSource,
    inspection_cache: &Arc<Mutex<RepoInspectionCache>>,
) -> Result<Vec<ForeachBinding>, String> {
    if bindings.0.is_empty() {
        return Err("the first !foreach item must contain a binding".to_string());
    }
    let mut resolved = Vec::with_capacity(bindings.0.len());
    for (name, choices) in bindings.0 {
        let choices = match choices {
            BindingChoices::Many(choices) => choices,
            BindingChoices::One(choice) => vec![choice],
        };
        let mut expanded = Vec::new();
        for mut choice in choices {
            if let Some(inherited) = inherited {
                choice = super::template::render_value(
                    choice,
                    inherited,
                    root,
                    config_path,
                    source,
                    inspection_cache,
                )?;
            }
            if let Some(glob) = choice.as_str().filter(|value| is_glob(value)) {
                let mut inspection_cache = inspection_cache
                    .lock()
                    .map_err(|_| "config inspection cache lock is poisoned".to_string())?;
                let paths = source.foreach_paths(&mut inspection_cache, root, config_path, glob)?;
                expanded.extend(paths.into_iter().map(Value::from));
            } else {
                expanded.push(choice);
            }
        }
        resolved.push(ForeachBinding {
            name,
            choices: expanded,
        });
    }
    Ok(resolved)
}

fn is_glob(value: &str) -> bool {
    value.contains(['*', '?'])
}
