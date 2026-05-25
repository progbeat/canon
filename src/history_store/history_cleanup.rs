use crate::check_selection::ExpectationIdentity;
#[cfg(test)]
use crate::config_types::CheckConfig;
#[cfg(test)]
use crate::hash::expectation_id;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[cfg(test)]
pub(crate) fn active_expectation_ids(config: &CheckConfig) -> BTreeSet<String> {
    config
        .expectations
        .iter()
        .map(|expectation| expectation_id(&expectation.q, &expectation.a))
        .collect()
}

pub(crate) fn active_expectation_ids_from_identities(
    identities: &[ExpectationIdentity],
) -> BTreeSet<String> {
    identities
        .iter()
        .map(|identity| identity.id.clone())
        .collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CacheCleanupStats {
    pub(crate) sampled: bool,
    pub(crate) removed: usize,
    pub(crate) kept: usize,
}

pub(crate) fn cleanup_stale_cache_dirs(
    cache_dir: &Path,
    active_ids: &BTreeSet<String>,
) -> Result<CacheCleanupStats, String> {
    if !cache_dir.exists() {
        return Ok(CacheCleanupStats {
            sampled: true,
            removed: 0,
            kept: 0,
        });
    }
    let mut stats = CacheCleanupStats {
        sampled: true,
        removed: 0,
        kept: 0,
    };
    for entry in fs::read_dir(cache_dir)
        .map_err(|err| format!("failed to read {}: {}", cache_dir.display(), err))?
    {
        let entry =
            entry.map_err(|err| format!("failed to read {}: {}", cache_dir.display(), err))?;
        let file_name = entry.file_name();
        let Some(id) = file_name.to_str() else {
            remove_cache_entry(&entry.path())?;
            stats.removed += 1;
            continue;
        };
        if active_ids.contains(id) {
            stats.kept += 1;
        } else {
            remove_cache_entry(&entry.path())?;
            stats.removed += 1;
        }
    }
    Ok(stats)
}

pub(crate) fn remove_cache_entry(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        fs::remove_dir_all(path)
            .map_err(|err| format!("failed to remove {}: {}", path.display(), err))
    } else {
        fs::remove_file(path).map_err(|err| format!("failed to remove {}: {}", path.display(), err))
    }
}
