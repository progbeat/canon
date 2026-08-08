use super::super::expansion::CheckConfigSource;
use crate::repo_inspection::RepoInspectionCache;
use minijinja::value::{Value, ValueKind};
use minijinja::Environment;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub(super) fn render_value(
    value: Value,
    combination: &BTreeMap<String, Value>,
    root: &Path,
    config_path: &Path,
    source: &CheckConfigSource,
    inspection_cache: &Arc<Mutex<RepoInspectionCache>>,
) -> Result<Value, String> {
    match value.kind() {
        ValueKind::String => render_string(
            value.as_str().unwrap_or_default(),
            combination,
            root,
            config_path,
            source,
            inspection_cache,
        )
        .map(Value::from),
        ValueKind::Seq => value
            .try_iter()
            .map_err(|err| format!("!foreach template: {err}"))?
            .map(|item| {
                render_value(
                    item,
                    combination,
                    root,
                    config_path,
                    source,
                    inspection_cache,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::from),
        ValueKind::Map => {
            let keys = value
                .try_iter()
                .map_err(|err| format!("!foreach template: {err}"))?
                .collect::<Vec<_>>();
            let mut rendered = BTreeMap::new();
            for key in keys {
                let item = value
                    .get_item(&key)
                    .map_err(|err| format!("!foreach template: {err}"))?;
                let key = render_value(
                    key,
                    combination,
                    root,
                    config_path,
                    source,
                    inspection_cache,
                )?;
                let item = render_value(
                    item,
                    combination,
                    root,
                    config_path,
                    source,
                    inspection_cache,
                )?;
                if rendered.insert(key, item).is_some() {
                    return Err("duplicate mapping key after !foreach rendering".to_string());
                }
            }
            Ok(Value::from(rendered))
        }
        _ => Ok(value),
    }
}

pub(super) fn render_string(
    value: &str,
    combination: &BTreeMap<String, Value>,
    root: &Path,
    config_path: &Path,
    source: &CheckConfigSource,
    inspection_cache: &Arc<Mutex<RepoInspectionCache>>,
) -> Result<String, String> {
    let mut environment = Environment::new();
    environment.set_keep_trailing_newline(true);
    add_foreach_read_function(
        &mut environment,
        root,
        config_path,
        source,
        inspection_cache,
    );
    let template = environment
        .template_from_str(value)
        .map_err(|err| format!("!foreach template: {err}"))?;
    template
        .render(combination)
        .map_err(|err| format!("!foreach template: {err}"))
}

fn add_foreach_read_function(
    environment: &mut Environment<'_>,
    root: &Path,
    config_path: &Path,
    source: &CheckConfigSource,
    inspection_cache: &Arc<Mutex<RepoInspectionCache>>,
) {
    let root = root.to_path_buf();
    let config_path = config_path.to_path_buf();
    let source = source.clone();
    let inspection_cache = Arc::clone(inspection_cache);
    environment.add_function("read", move |requested: String| {
        let mut inspection_cache = inspection_cache.lock().map_err(|_| {
            minijinja::Error::new(
                minijinja::ErrorKind::InvalidOperation,
                "config inspection cache lock is poisoned",
            )
        })?;
        source
            .foreach_literal_file_content(&mut inspection_cache, &root, &config_path, &requested)
            .map_err(|err| {
                minijinja::Error::new(
                    minijinja::ErrorKind::InvalidOperation,
                    format!("!foreach read({requested:?}): {err}"),
                )
            })
    });
}
