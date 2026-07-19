use crate::check::ExpectationIdentity;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub(crate) fn prune_uncollected_xpec_state_dirs(
    xpecs_dir: &Path,
    collected_ids: &BTreeSet<String>,
) -> Result<XpecStateRetentionStats, String> {
    if !xpecs_dir.exists() {
        return Ok(XpecStateRetentionStats {
            removed: 0,
            kept: 0,
        });
    }
    let mut stats = XpecStateRetentionStats {
        removed: 0,
        kept: 0,
    };
    for entry in fs::read_dir(xpecs_dir)
        .map_err(|err| format!("failed to read {}: {}", xpecs_dir.display(), err))?
    {
        let entry =
            entry.map_err(|err| format!("failed to read {}: {}", xpecs_dir.display(), err))?;
        let file_name = entry.file_name();
        let Some(id) = file_name.to_str() else {
            remove_state_entry(&entry.path())?;
            stats.removed += 1;
            continue;
        };
        if collected_ids.contains(id) {
            stats.kept += 1;
        } else {
            remove_state_entry(&entry.path())?;
            stats.removed += 1;
        }
    }
    Ok(stats)
}

pub(crate) fn collected_expectation_ids_from_identities(
    identities: &[ExpectationIdentity],
) -> BTreeSet<String> {
    identities
        .iter()
        .map(|identity| identity.id.clone())
        .collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct XpecStateRetentionStats {
    pub(crate) removed: usize,
    pub(crate) kept: usize,
}

fn remove_state_entry(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        fs::remove_dir_all(path)
            .map_err(|err| format!("failed to remove {}: {}", path.display(), err))
    } else {
        fs::remove_file(path).map_err(|err| format!("failed to remove {}: {}", path.display(), err))
    }
}

#[cfg(test)]
mod tests {
    use super::prune_uncollected_xpec_state_dirs;
    use std::collections::BTreeSet;
    use std::fs;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: fh,Ijl
    fn retention_prunes_uncollected_entries_and_preserves_collected_xpec_history() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("canon-cleanup-{}-{unique}", process::id()));
        let active_dir = root.join("active");
        let stale_dir = root.join("stale");
        fs::create_dir_all(&active_dir).unwrap();
        fs::create_dir_all(&stale_dir).unwrap();
        fs::write(active_dir.join("last-pass.json"), "{}\n").unwrap();
        fs::write(stale_dir.join("last-fail.json"), "{}\n").unwrap();
        let collected_ids = BTreeSet::from(["active".to_string()]);

        let stats = prune_uncollected_xpec_state_dirs(&root, &collected_ids).unwrap();

        assert_eq!(stats.removed, 1);
        assert_eq!(stats.kept, 1);
        assert!(active_dir.join("last-pass.json").is_file());
        assert!(!stale_dir.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
