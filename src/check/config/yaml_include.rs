use super::expansion::CheckConfigSource;
use super::foreach::expand_current_expectation_items;
use crate::config_types::{RawCheckConfig, RawLegacyAgentConfig, RawPresetConfig};
use crate::repo_inspection::RepoInspectionCache;
use crate::scope::normalize_repo_path;
use serde::de::DeserializeOwned;
use serde::de::IgnoredAny;
use serde::Deserialize;
use serde_saphyr::{
    from_str_with_options, IncludeRequest, IncludeResolveError, InputSource, ResolvedInclude,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// Shared by top-level check config loading and recursive expectation includes;
// both paths need the same source-aware YAML `!include` resolver.
#[cfg(test)]
pub(crate) fn parse_yaml_config_with_includes<T>(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: CheckConfigSource,
) -> Result<T, String>
where
    T: DeserializeOwned,
{
    parse_yaml_config_with_includes_and_cache(
        root,
        config_path,
        content,
        source,
        RepoInspectionCache::new(),
    )
}

#[cfg(test)]
pub(crate) fn parse_yaml_config_with_includes_and_cache<T>(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: CheckConfigSource,
    inspection_cache: RepoInspectionCache,
) -> Result<T, String>
where
    T: DeserializeOwned,
{
    parse_yaml_config_with_shared_cache(
        root,
        config_path,
        content,
        source,
        Arc::new(Mutex::new(inspection_cache)),
    )
}

pub(crate) fn parse_raw_check_config_with_includes_and_foreach(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: CheckConfigSource,
    inspection_cache: RepoInspectionCache,
) -> Result<RawCheckConfig, String> {
    let inspection_cache = Arc::new(Mutex::new(inspection_cache));
    let header: RawCheckConfigHeader = parse_yaml_config_with_shared_cache(
        root,
        config_path,
        content,
        source.clone(),
        Arc::clone(&inspection_cache),
    )?;
    let expectations =
        expand_current_expectation_items(root, config_path, content, &source, inspection_cache)?;
    Ok(RawCheckConfig {
        version: header.version,
        presets: header.presets,
        agent: header.agent,
        expectations,
    })
}

fn parse_yaml_config_with_shared_cache<T>(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: CheckConfigSource,
    inspection_cache: Arc<Mutex<RepoInspectionCache>>,
) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let mut resolver = CheckConfigIncludeResolver {
        root: root.to_path_buf(),
        root_config_path: config_path.to_path_buf(),
        source: source.clone(),
        inspection_cache,
    };
    let options = serde_saphyr::options! {
        strict_booleans: true,
    }
    .with_include_resolver(move |request: IncludeRequest<'_>| resolver.resolve(request));
    from_str_with_options(content, options).map_err(|err| err.to_string())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCheckConfigHeader {
    #[serde(default = "default_config_version")]
    version: u32,
    #[serde(default)]
    presets: Option<BTreeMap<String, RawPresetConfig>>,
    #[serde(default)]
    agent: Option<RawLegacyAgentConfig>,
    #[serde(rename = "expectations", alias = "xpecs")]
    _expectations: IgnoredAny,
}

fn default_config_version() -> u32 {
    1
}

struct CheckConfigIncludeResolver {
    root: PathBuf,
    root_config_path: PathBuf,
    source: CheckConfigSource,
    inspection_cache: Arc<Mutex<RepoInspectionCache>>,
}

impl CheckConfigIncludeResolver {
    fn resolve(
        &mut self,
        request: IncludeRequest<'_>,
    ) -> Result<ResolvedInclude, IncludeResolveError> {
        let path = resolve_include_path(&self.root_config_path, request.spec, request.from_id)
            .map_err(IncludeResolveError::Message)?;
        // `serde_saphyr` rejects non-root cycles by comparing each
        // `ResolvedInclude.id` against the active include IDs. The root input
        // has no ID in that stack, so `resolve_include_path` rejects only the
        // root-file cycle that the parser cannot observe itself.
        let content = {
            let mut inspection_cache = self.inspection_cache.lock().map_err(|_| {
                IncludeResolveError::Message("config inspection cache lock is poisoned".to_string())
            })?;
            self.source
                .file_content(&mut inspection_cache, &self.root, Path::new(&path))
                .map_err(IncludeResolveError::Message)?
        };
        Ok(ResolvedInclude {
            id: path.clone(),
            name: path,
            source: InputSource::Text(content),
        })
    }
}

pub(super) fn resolve_include_path(
    root_config_path: &Path,
    spec: &str,
    from_id: Option<&str>,
) -> Result<String, String> {
    let including_path = including_path(root_config_path, from_id);
    let include_path = normalize_include_spec(spec)?;
    let base = including_path.parent().unwrap_or_else(|| Path::new(""));
    let joined = base.join(include_path);
    let joined = joined
        .to_str()
        .ok_or_else(|| format!("include path must be valid UTF-8: {spec}"))?;
    normalize_repo_path(joined)
        .map_err(|err| format!("include path: {err}"))
        .and_then(|path| reject_root_include(&path, root_config_path))
}

fn including_path(root_config_path: &Path, from_id: Option<&str>) -> PathBuf {
    match from_id {
        Some(path) => PathBuf::from(path),
        None => root_config_path.to_path_buf(),
    }
}

fn normalize_include_spec(spec: &str) -> Result<String, String> {
    if spec.is_empty() {
        return Err("include path must not be empty".to_string());
    }
    let path = normalize_repo_path(spec).map_err(|err| format!("include path: {err}"))?;
    if path == "." {
        return Err("include path must name a file".to_string());
    }
    Ok(path)
}

fn reject_root_include(path: &str, root_config_path: &Path) -> Result<String, String> {
    let root_config_path = root_config_path
        .to_str()
        .ok_or_else(|| "root config path must be valid UTF-8".to_string())?;
    let root_config_path =
        normalize_repo_path(root_config_path).map_err(|err| format!("root config path: {err}"))?;
    if path == root_config_path {
        return Err(format!("recursive YAML include: {path}"));
    }
    Ok(path.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_include_spec, parse_yaml_config_with_includes, reject_root_include,
        resolve_include_path,
    };
    use crate::check::config::expansion::CheckConfigSource;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: I8,MH
    fn expectation_sequence_includes_are_flattened() {
        let root = test_root("expectation-sequence-include");
        fs::write(
            root.join("included.yml"),
            "- q: Included one\n  a: yes\n- q: Included two\n  a: yes\n",
        )
        .unwrap();

        let raw = parse_yaml_config_with_includes::<crate::config_types::RawCheckConfig>(
            &root,
            Path::new("check.yml"),
            "presets:\n  default: {}\nxpecs:\n  - !include included.yml\n  - q: Local\n    a: yes\n",
            CheckConfigSource::InPlace,
        )
        .unwrap();

        assert_eq!(raw.expectations.len(), 3);
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: I8,1H
    fn included_presets_preserve_fields_from_every_yaml_merge_source() {
        let root = test_root("included-preset-yaml-merge");
        fs::write(
            root.join("presets.yml"),
            "\
default: &default
  ignore: [generated/**]
capability: &capability
  models: [review-model]
review-base: &review-base
  <<: *default
  target: diff
patch-review:
  <<: [*capability, *review-base]
",
        )
        .unwrap();
        let raw = parse_yaml_config_with_includes::<crate::config_types::RawCheckConfig>(
            &root,
            Path::new("check.yml"),
            "presets: !include presets.yml\nxpecs: []\n",
            CheckConfigSource::InPlace,
        )
        .unwrap();

        let patch_review = &raw.presets.unwrap()["patch-review"];
        assert_eq!(patch_review.target.value.as_deref(), Some("diff"));
        assert_eq!(
            patch_review.ignore.value.as_deref(),
            Some(["generated/**".to_string()].as_slice())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: MH
    fn exactly_one_of_xpecs_or_expectations_is_required() {
        for key in ["xpecs", "expectations"] {
            let content = format!("presets:\n  default: {{}}\n{key}:\n  - q: Local\n    a: yes\n");
            let raw = parse_yaml_config_with_includes::<crate::config_types::RawCheckConfig>(
                Path::new("."),
                Path::new("check.yml"),
                &content,
                CheckConfigSource::InPlace,
            )
            .unwrap();
            assert_eq!(raw.expectations.len(), 1);
        }

        assert!(
            parse_yaml_config_with_includes::<crate::config_types::RawCheckConfig>(
                Path::new("."),
                Path::new("check.yml"),
                "presets:\n  default: {}\nxpecs: []\nexpectations: []\n",
                CheckConfigSource::InPlace,
            )
            .is_err()
        );

        assert!(
            parse_yaml_config_with_includes::<crate::config_types::RawCheckConfig>(
                Path::new("."),
                Path::new("check.yml"),
                "presets:\n  default: {}\n",
                CheckConfigSource::InPlace,
            )
            .is_err()
        );
    }

    #[test] // xpec: MH
    fn nested_xpec_sequences_are_recursively_flattened() {
        let raw = parse_yaml_config_with_includes::<crate::config_types::RawCheckConfig>(
            Path::new("."),
            Path::new("check.yml"),
            "presets:\n  default: {}\nxpecs:\n  - - q: One\n      a: yes\n    - - q: Two\n        a: yes\n",
            CheckConfigSource::InPlace,
        )
        .unwrap();

        assert_eq!(raw.expectations.len(), 2);
    }

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

    // xpec: T5
    #[test]
    fn invalid_root_config_path_does_not_bypass_root_include_rejection() {
        // xpec: T5
        assert!(reject_root_include("check.yml", Path::new("../check.yml")).is_err());
    }

    // xpec: T5,I8
    #[test]
    fn yaml_include_non_root_cycle_is_rejected_by_resolved_include_ids() {
        let root = test_root("yaml-include-non-root-cycle");
        fs::write(root.join("child.yml"), "child: !include sibling.yml\n").unwrap();
        fs::write(root.join("sibling.yml"), "sibling: !include child.yml\n").unwrap();

        let err = parse_yaml_config_with_includes::<serde_json::Value>(
            &root,
            Path::new("check.yml"),
            "root: !include child.yml\n",
            CheckConfigSource::InPlace,
        )
        .unwrap_err();

        // xpec: T5,I8
        assert!(
            err.contains("cyclic include detected: child.yml"),
            "unexpected include error: {err}"
        );
    }

    fn test_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("canon-test-{}-{}-{}", name, process::id(), unique));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }
}
