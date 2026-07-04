use super::config_expansion::CheckConfigSource;
use crate::repo_inspection::RepoInspectionCache;
use crate::scope::normalize_repo_path;
use serde::de::DeserializeOwned;
use serde_saphyr::{
    from_str_with_options, IncludeRequest, IncludeResolveError, InputSource, ResolvedInclude,
};
use std::path::{Path, PathBuf};

// Shared by top-level check config loading and recursive expectation includes;
// both paths need the same source-aware YAML `!include` resolver.
pub(crate) fn parse_yaml_config_with_includes<T>(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: CheckConfigSource,
) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let mut resolver = CheckConfigIncludeResolver {
        root: root.to_path_buf(),
        root_config_path: config_path.to_path_buf(),
        source,
        cache: RepoInspectionCache::new(),
    };
    let options = serde_saphyr::options! {
        strict_booleans: true,
    }
    .with_include_resolver(move |request: IncludeRequest<'_>| resolver.resolve(request));
    from_str_with_options(content, options).map_err(|err| err.to_string())
}

struct CheckConfigIncludeResolver {
    root: PathBuf,
    root_config_path: PathBuf,
    source: CheckConfigSource,
    cache: RepoInspectionCache,
}

impl CheckConfigIncludeResolver {
    fn resolve(
        &mut self,
        request: IncludeRequest<'_>,
    ) -> Result<ResolvedInclude, IncludeResolveError> {
        let path = resolve_include_path(&self.root_config_path, request.spec, request.from_id)?;
        let content = self
            .cache
            .config_source_file_content(&self.root, &self.source, Path::new(&path))
            .map_err(IncludeResolveError::Message)?;
        Ok(ResolvedInclude {
            id: path.clone(),
            name: path,
            source: InputSource::Text(content),
        })
    }
}

fn resolve_include_path(
    root_config_path: &Path,
    spec: &str,
    from_id: Option<&str>,
) -> Result<String, IncludeResolveError> {
    let including_path = including_path(root_config_path, from_id);
    let include_path = normalize_include_spec(spec)?;
    let base = including_path.parent().unwrap_or_else(|| Path::new(""));
    let joined = base.join(include_path);
    let joined = joined
        .to_str()
        .ok_or_else(|| include_error(format!("include path must be valid UTF-8: {spec}")))?;
    normalize_repo_path(joined)
        .map_err(|err| include_error(format!("include path: {err}")))
        .and_then(|path| reject_root_include(&path, root_config_path))
}

fn including_path(root_config_path: &Path, from_id: Option<&str>) -> PathBuf {
    match from_id {
        Some(path) => PathBuf::from(path),
        None => root_config_path.to_path_buf(),
    }
}

fn normalize_include_spec(spec: &str) -> Result<String, IncludeResolveError> {
    if spec.is_empty() {
        return Err(include_error("include path must not be empty"));
    }
    let path =
        normalize_repo_path(spec).map_err(|err| include_error(format!("include path: {err}")))?;
    if path == "." {
        return Err(include_error("include path must name a file"));
    }
    Ok(path)
}

fn reject_root_include(path: &str, root_config_path: &Path) -> Result<String, IncludeResolveError> {
    let Some(root_config_path) = root_config_path.to_str() else {
        return Ok(path.to_string());
    };
    let Ok(root_config_path) = normalize_repo_path(root_config_path) else {
        return Ok(path.to_string());
    };
    if path == root_config_path {
        return Err(include_error(format!("recursive YAML include: {path}")));
    }
    Ok(path.to_string())
}

fn include_error(message: impl Into<String>) -> IncludeResolveError {
    IncludeResolveError::Message(message.into())
}

#[cfg(test)]
mod tests {
    use super::{normalize_include_spec, parse_yaml_config_with_includes, resolve_include_path};
    use std::path::Path;

    // xpec: I8
    #[test]
    fn include_paths_resolve_relative_to_including_file() {
        let path = resolve_include_path(Path::new(".canon/check.yml"), "hooks/on-start.yml", None)
            .unwrap();

        // xpec: I8
        assert_eq!(path, ".canon/hooks/on-start.yml");
    }

    // xpec: I8
    #[test]
    fn nested_include_paths_resolve_relative_to_parent_include() {
        let path = resolve_include_path(
            Path::new(".canon/check.yml"),
            "shared.yml",
            Some(".canon/hooks/on-start.yml"),
        )
        .unwrap();

        // xpec: I8
        assert_eq!(path, ".canon/hooks/shared.yml");
    }

    // xpec: I8
    #[test]
    fn unsafe_include_paths_are_rejected() {
        for spec in ["", ".", "/abs.yml", "../parent.yml", "nested/../parent.yml"] {
            // xpec: I8
            assert!(
                normalize_include_spec(spec).is_err(),
                "expected unsafe include path to fail: {spec}"
            );
        }
    }

    // xpec: uY
    #[test]
    fn hook_case_key_y_stays_text() {
        let raw: crate::config_types::RawCheckConfig = parse_yaml_config_with_includes(
            Path::new("."),
            Path::new("check.yml"),
            r#"
version: 1
presets:
  default: {}
hooks:
  on-start:
    input: "Continue? "
    cases:
      y: !ok
      _: !block "Stop."
expectations:
  - q: "Does hook config parse?"
    a: "yes"
"#,
            crate::check::config::CheckConfigSource::InPlace,
        )
        .expect("parse hook config");

        let hooks = raw.hooks.unwrap().resolve().unwrap();

        // xpec: uY
        assert!(hooks.on_start[0].cases.contains_key("y"));
        // xpec: uY
        assert!(!hooks.on_start[0].cases.contains_key("true"));
    }
}
