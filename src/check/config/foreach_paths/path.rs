use std::path::{Component, Path};

pub(crate) fn resolve_foreach_read_path(config_path: &Path, path: &str) -> Result<String, String> {
    resolve_foreach_path(config_path, path, "foreach read path")
}

pub(super) struct ResolvedForeachGlob {
    literal_prefix: Vec<String>,
    pattern: String,
}

impl ResolvedForeachGlob {
    pub(super) fn pattern(&self) -> &str {
        &self.pattern
    }

    pub(super) fn utf8_candidate_suffix(&self, path: &Path) -> Result<Option<String>, String> {
        let path = normal_components(path, "foreach matched path")?;
        if !path.starts_with(&self.literal_prefix) {
            return Ok(None);
        }
        Ok(Some(join_components(&path[self.literal_prefix.len()..])))
    }

    pub(super) fn byte_candidate_suffix(&self, path: &[u8]) -> Option<Vec<u8>> {
        let path = path.split(|byte| *byte == b'/').collect::<Vec<_>>();
        let prefix_matches = self
            .literal_prefix
            .iter()
            .zip(&path)
            .all(|(literal, component)| literal.as_bytes() == *component);
        if !prefix_matches || path.len() < self.literal_prefix.len() {
            return None;
        }
        let suffix = &path[self.literal_prefix.len()..];
        if suffix.is_empty() {
            Some(b".".to_vec())
        } else {
            Some(suffix.join(&b'/'))
        }
    }
}

pub(super) fn resolve_foreach_glob(
    config_path: &Path,
    glob: &str,
) -> Result<ResolvedForeachGlob, String> {
    let target = resolve_foreach_components(config_path, glob, "foreach path glob", true)?;
    let first_pattern = target
        .iter()
        .position(|(_, is_pattern)| *is_pattern)
        .unwrap_or(target.len());
    Ok(ResolvedForeachGlob {
        literal_prefix: target[..first_pattern]
            .iter()
            .map(|(component, _)| component.clone())
            .collect(),
        pattern: join_components(
            &target[first_pattern..]
                .iter()
                .map(|(component, _)| component.clone())
                .collect::<Vec<_>>(),
        ),
    })
}

fn resolve_foreach_path(config_path: &Path, path: &str, label: &str) -> Result<String, String> {
    let parts = resolve_foreach_components(config_path, path, label, false)?;
    Ok(join_components(
        &parts
            .into_iter()
            .map(|(component, _)| component)
            .collect::<Vec<_>>(),
    ))
}

fn resolve_foreach_components(
    config_path: &Path,
    path: &str,
    label: &str,
    glob_semantics: bool,
) -> Result<Vec<(String, bool)>, String> {
    if path.is_empty() {
        return Err(format!("{label}: path must not be empty"));
    }
    if path.contains('\0') {
        return Err(format!("{label}: path must not contain NUL bytes"));
    }
    let relative = Path::new(path);
    if relative.is_absolute() {
        return Err(format!("{label}: path must be relative: {path}"));
    }
    let mut parts = normal_components(
        config_path.parent().unwrap_or_else(|| Path::new("")),
        &format!("{label}: config path"),
    )?
    .into_iter()
    .map(|component| (component, false))
    .collect::<Vec<_>>();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| format!("{label}: path must be valid UTF-8: {path}"))?;
                parts.push((
                    part.to_string(),
                    glob_semantics && part.contains(['*', '?']),
                ));
            }
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(format!("{label}: path escapes the source: {path}"));
                }
            }
            _ => return Err(format!("{label}: unsupported path component in {path}")),
        }
    }
    Ok(parts)
}

pub(super) fn path_relative_to_config(config_path: &Path, path: &Path) -> Result<String, String> {
    let config_dir = normal_components(
        config_path.parent().unwrap_or_else(|| Path::new("")),
        "foreach config path",
    )?;
    let path = normal_components(path, "foreach matched path")?;
    Ok(relative_components(&config_dir, &path))
}

fn relative_components(config_dir: &[String], path: &[String]) -> String {
    let common = config_dir
        .iter()
        .zip(path)
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = vec!["..".to_string(); config_dir.len() - common];
    relative.extend(path[common..].iter().cloned());
    join_components(&relative)
}

fn join_components(parts: &[String]) -> String {
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

fn normal_components(path: &Path, label: &str) -> Result<Vec<String>, String> {
    let mut normal = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err(format!("{label} must be relative and normalized"));
        };
        let part = part
            .to_str()
            .ok_or_else(|| format!("{label} must be valid UTF-8"))?;
        normal.push(part.to_string());
    }
    Ok(normal)
}

#[cfg(all(test, unix))]
mod tests {
    use super::{path_relative_to_config, resolve_foreach_read_path};
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::{Path, PathBuf};

    #[test] // xpec: gO
    fn foreach_path_transformations_reject_non_utf8_components() {
        let non_utf8_config = PathBuf::from(OsString::from_vec(b".canon/\xff/check.yml".to_vec()));
        let non_utf8_match = PathBuf::from(OsString::from_vec(b"specs/\xff.md".to_vec()));

        assert_eq!(
            resolve_foreach_read_path(&non_utf8_config, "spec.md").unwrap_err(),
            "foreach read path: config path must be valid UTF-8"
        );
        assert_eq!(
            path_relative_to_config(Path::new(".canon/check.yml"), &non_utf8_match).unwrap_err(),
            "foreach matched path must be valid UTF-8"
        );
        assert_eq!(
            resolve_foreach_read_path(Path::new("../check.yml"), "spec.md").unwrap_err(),
            "foreach read path: config path must be relative and normalized"
        );
    }
}
