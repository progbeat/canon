use super::StagedWorktreeView;
use crate::git::{git_object_oid_has_known_shape, StagedTrackedFile};
use std::collections::BTreeMap;

pub(super) struct VisibleTree {
    pub(super) oid: String,
    pub(super) entry_paths: Vec<StagedTrackedFile>,
}

pub(super) struct VisibleTreeChild {
    pub(super) name: Vec<u8>,
    pub(super) path: Vec<u8>,
    pub(super) is_dir: bool,
}

impl StagedWorktreeView {
    pub(super) fn visible_tree(
        &self,
        scope: &[String],
        visible_tree_oid: &str,
    ) -> Result<VisibleTree, String> {
        if !git_object_oid_has_known_shape(visible_tree_oid) {
            return Err("visibleTreeOid must be a Git object ID hex string".to_string());
        }
        let entry_paths = self
            .source
            .tracked_files_for_pathspecs(&self.source_root, scope)?;
        Ok(VisibleTree {
            oid: visible_tree_oid.to_string(),
            entry_paths,
        })
    }
}

impl VisibleTree {
    pub(super) fn children(&self, prefix: &[u8]) -> Vec<VisibleTreeChild> {
        let prefix_components = path_components(prefix);
        let mut children = BTreeMap::new();
        for file in &self.entry_paths {
            let components = path_components(&file.path);
            if !components.starts_with(&prefix_components)
                || components.len() == prefix_components.len()
            {
                continue;
            }
            let child_components = &components[..prefix_components.len() + 1];
            let child_path = join_path_components(child_components);
            let is_leaf = child_components.len() == components.len();
            let is_dir = !is_leaf;
            children
                .entry(child_path.clone())
                .or_insert_with(|| VisibleTreeChild {
                    name: child_components
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .to_vec(),
                    path: child_path,
                    is_dir,
                });
        }
        children.into_values().collect()
    }
}

fn path_components(path: &[u8]) -> Vec<&[u8]> {
    path.split(|byte| *byte == b'/')
        .filter(|component| !component.is_empty())
        .collect()
}

fn join_path_components(components: &[&[u8]]) -> Vec<u8> {
    let mut path = Vec::new();
    for component in components {
        if !path.is_empty() {
            path.push(b'/');
        }
        path.extend_from_slice(component);
    }
    path
}
