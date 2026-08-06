//! Host paths hidden from materialized evaluator sessions.

use super::super::permissions::{insert_filesystem_permission, FILESYSTEM_DENY};
use super::super::{EvaluatorConfigError, EvaluatorConfigResult};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::{Path, PathBuf};

const PROTECTED_HOST_ENVIRONMENT_PATHS: [&str; 4] =
    ["HOME", "CODEX_HOME", "XDG_RUNTIME_DIR", "GNUPGHOME"];

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct EvaluatorHostIsolation {
    protected_roots: BTreeSet<PathBuf>,
}

impl EvaluatorHostIsolation {
    pub(crate) fn for_project(project_root: &Path) -> Result<EvaluatorHostIsolation, String> {
        let project_root = project_root.canonicalize().map_err(|err| {
            format!(
                "failed to canonicalize evaluator source project root {}: {}",
                project_root.display(),
                err
            )
        })?;
        let mut environment_roots = Vec::new();
        for name in PROTECTED_HOST_ENVIRONMENT_PATHS {
            let Some(value) = env::var_os(name).filter(|value| !value.is_empty()) else {
                continue;
            };
            if let Ok(path) = PathBuf::from(value).canonicalize() {
                environment_roots.push(path);
            }
        }
        Ok(EvaluatorHostIsolation {
            protected_roots: minimal_protected_host_roots(project_root, environment_roots)
                .into_iter()
                .collect(),
        })
    }

    pub(crate) fn in_place() -> EvaluatorHostIsolation {
        EvaluatorHostIsolation {
            protected_roots: BTreeSet::new(),
        }
    }

    pub(super) fn read_denials(
        &self,
        evaluator_read_roots: &[&Path],
    ) -> EvaluatorConfigResult<BTreeMap<String, String>> {
        evaluator_host_read_denials(&self.protected_roots, evaluator_read_roots)
    }

    #[cfg(test)]
    pub(in crate::evaluator::config::runtime) fn from_protected_roots(
        protected_roots: impl IntoIterator<Item = PathBuf>,
    ) -> EvaluatorHostIsolation {
        EvaluatorHostIsolation {
            protected_roots: protected_roots.into_iter().collect(),
        }
    }
}

fn minimal_protected_host_roots(
    project_root: PathBuf,
    environment_roots: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let minimal_environment_roots = minimal_path_roots(environment_roots);
    let mut roots = vec![project_root.clone()];
    roots.extend(
        minimal_environment_roots
            .into_iter()
            .filter(|root| root != &project_root && !root.starts_with(&project_root)),
    );
    roots.sort();
    roots
}

fn evaluator_host_read_denials<'a>(
    protected_host_roots: impl IntoIterator<Item = &'a PathBuf>,
    evaluator_read_roots: &[&Path],
) -> EvaluatorConfigResult<BTreeMap<String, String>> {
    let roots = protected_host_roots
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    for root in &roots {
        if let Some(read_root) = evaluator_read_roots
            .iter()
            .find(|read_root| read_root.starts_with(root))
        {
            return Err(EvaluatorConfigError::RuntimeInputInsideProtectedHostRoot {
                input: (*read_root).to_path_buf(),
                protected_root: root.clone(),
            });
        }
    }
    let minimal_roots = minimal_path_roots(roots);

    let mut permissions = BTreeMap::new();
    for root in minimal_roots {
        let root = super::super::path_to_config_string(&root, "protected evaluator host root")?;
        insert_filesystem_permission(&mut permissions, root, FILESYSTEM_DENY)?;
    }
    Ok(permissions)
}

fn minimal_path_roots(mut roots: Vec<PathBuf>) -> Vec<PathBuf> {
    roots.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    let mut minimal_roots = Vec::new();
    for root in roots {
        if !minimal_roots
            .iter()
            .any(|ancestor: &PathBuf| root.starts_with(ancestor))
        {
            minimal_roots.push(root);
        }
    }
    minimal_roots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolation(roots: &[&str]) -> EvaluatorHostIsolation {
        EvaluatorHostIsolation::from_protected_roots(roots.iter().map(PathBuf::from))
    }

    #[test] // xpec: A8,8,vr,bP
    fn host_denials_preserve_disjoint_evaluator_inputs() {
        let permissions = isolation(&["/host/home", "/host/project"])
            .read_denials(&[
                Path::new("/runtime/codex"),
                Path::new("/sandbox/tree"),
                Path::new("/artifacts"),
            ])
            .unwrap();

        assert_eq!(
            permissions,
            BTreeMap::from([
                ("/host/home".to_string(), FILESYSTEM_DENY.to_string()),
                ("/host/project".to_string(), FILESYSTEM_DENY.to_string()),
            ])
        );
    }

    #[test] // xpec: KD
    fn host_denials_reject_runtime_inputs_inside_protected_roots() {
        let error = isolation(&["/tmp"])
            .read_denials(&[Path::new("/tmp/runtime/codex")])
            .unwrap_err();

        assert!(matches!(
            error,
            EvaluatorConfigError::RuntimeInputInsideProtectedHostRoot {
                input,
                protected_root,
            } if input == Path::new("/tmp/runtime/codex")
                && protected_root == Path::new("/tmp")
        ));
    }

    #[test] // xpec: A8,KD
    fn host_denials_remove_nested_mounts_below_an_active_deny() {
        let permissions = isolation(&["/host/home", "/host/home/project"])
            .read_denials(&[Path::new("/runtime/codex")])
            .unwrap();

        assert_eq!(
            permissions,
            BTreeMap::from([("/host/home".to_string(), FILESYSTEM_DENY.to_string())])
        );
    }
}
