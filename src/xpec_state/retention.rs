use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use super::gate_history::{preserve_canonical_results, CACHE_FILE_NAME};

pub(crate) fn prune_uncollected_xpec_state_dirs(
    xpecs_dir: &Path,
    collected_ids: &BTreeSet<String>,
) -> Result<XpecStateRetentionStats, String> {
    // [fh] This exhaustive directory sweep is what prevents full IDs from
    // accumulating across configuration changes: every entry not present in
    // the current resolved configuration is removed on the next normal check.
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

pub(crate) fn prune_uncollected_in_place_xpec_state(
    xpecs_dir: &Path,
    collected_ids: &BTreeSet<String>,
) -> Result<XpecStateRetentionStats, String> {
    // [fh,KD,Sh] In-place owns canonical Last Results for its complete
    // working-directory configuration, but not the bounded Git-backed cache
    // that lets gate classify already-evaluated Git trees. For an uncollected
    // ID, retain only that cache and remove every in-place-owned or stray
    // sibling entry.
    if !xpecs_dir.exists() {
        return Ok(XpecStateRetentionStats::default());
    }
    let mut stats = XpecStateRetentionStats::default();
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
            continue;
        }
        retain_only_gate_results(&entry.path())?;
        if entry.path().exists() {
            stats.kept += 1;
        } else {
            stats.removed += 1;
        }
    }
    Ok(stats)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct XpecStateRetentionStats {
    pub(crate) removed: usize,
    pub(crate) kept: usize,
}

fn remove_state_entry(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(path)
            .map_err(|err| format!("failed to remove {}: {}", path.display(), err))
    } else {
        fs::remove_file(path).map_err(|err| format!("failed to remove {}: {}", path.display(), err))
    }
}

fn retain_only_gate_results(xpec_path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(xpec_path)
        .map_err(|err| format!("failed to inspect {}: {}", xpec_path.display(), err))?;
    if !metadata.file_type().is_dir() {
        return remove_state_entry(xpec_path);
    }
    // [KD] Migrate canonical-only Git history before in-place retention
    // removes Last Results for an ID absent from the working configuration.
    preserve_canonical_results(xpec_path)?;
    for entry in fs::read_dir(xpec_path)
        .map_err(|err| format!("failed to read {}: {}", xpec_path.display(), err))?
    {
        let entry =
            entry.map_err(|err| format!("failed to read {}: {}", xpec_path.display(), err))?;
        if entry.file_name() != CACHE_FILE_NAME {
            remove_state_entry(&entry.path())?;
        }
    }
    if fs::read_dir(xpec_path)
        .map_err(|err| format!("failed to read {}: {}", xpec_path.display(), err))?
        .next()
        .is_none()
    {
        fs::remove_dir(xpec_path)
            .map_err(|err| format!("failed to remove {}: {}", xpec_path.display(), err))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::check::{CheckRecord, CheckResult, ExpectationIdentity};
    use crate::config_types::{AgentConfig, ExpectationTo, DEFAULT_DIFF_FROM};
    use crate::xpec_state::XpecStateCache;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: fh,Ijl
    fn retention_prunes_uncollected_entries_and_preserves_collected_xpec_history() {
        let root = git_project("retention").unwrap();
        let mut state = XpecStateCache::default();
        let active = expectation("active");
        let stale = expectation("stale");
        assert!(write_pass(&mut state, &root, &active, true).is_err());
        state
            .retain_only_current_configuration(&root, &[identity("active"), identity("stale")])
            .unwrap();
        write_pass(&mut state, &root, &active, true).unwrap();
        write_pass(&mut state, &root, &stale, true).unwrap();

        let stats = state
            .retain_only_current_configuration(&root, &[identity("active")])
            .unwrap();
        let mut observed = XpecStateCache::default();

        assert_eq!(stats, (1, 1));
        assert!(observed.read_last_pass(&root, &active).unwrap().is_some());
        assert!(observed.read_last_pass(&root, &stale).unwrap().is_none());
        assert!(write_pass(&mut state, &root, &stale, true).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: fh,Ijl,KD,Sh
    fn in_place_retention_preserves_only_git_backed_cache_for_uncollected_ids() {
        let root = git_project("in-place-retention").unwrap();
        let state_root = crate::state_paths::CanonStateRoot::resolve(&root).unwrap();
        let mut state = XpecStateCache::default();
        let active = expectation("active");
        let git_backed = expectation("git-backed");
        let stale = expectation("stale");
        state
            .retain_only_current_configuration(&root, &[identity("git-backed")])
            .unwrap();
        write_pass(&mut state, &root, &git_backed, true).unwrap();

        state.bind_in_place_state_root(&root, &state_root);
        state
            .retain_only_current_configuration(&root, &[identity("active"), identity("stale")])
            .unwrap();
        write_pass(&mut state, &root, &active, false).unwrap();
        write_pass(&mut state, &root, &stale, false).unwrap();

        let stats = state
            .retain_only_current_configuration(&root, &[identity("active")])
            .unwrap();
        let mut observed = XpecStateCache::default();

        assert_eq!(stats, (1, 2));
        assert!(observed.read_last_pass(&root, &active).unwrap().is_some());
        assert!(observed
            .read_gate_results(&root, &expectation("git-backed"))
            .unwrap()
            .is_some());
        assert!(observed
            .read_last_pass(&root, &expectation("git-backed"))
            .unwrap()
            .is_none());
        assert!(observed
            .read_gate_results(&root, &expectation("stale"))
            .unwrap()
            .is_none());
        assert!(observed.read_last_pass(&root, &stale).unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    fn write_pass(
        state: &mut XpecStateCache,
        root: &Path,
        expectation: &crate::check::ResolvedExpectation,
        git_backed: bool,
    ) -> Result<(), String> {
        let id = expectation.require_configured_id()?;
        let record = CheckRecord {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            result: CheckResult::Pass,
            to: ExpectationTo::Agent,
            question: Some(expectation.question.clone()),
            expected_answer: Some(expectation.expected_answer().to_string()),
            observed: expectation.expected_answer().to_string(),
            error: None,
            evidence: Some("test evidence".to_string()),
            scope: vec![".".to_string()],
            q_scope_suggestion: None,
            visible_tree_oid: git_backed.then(|| format!("{id}-visible-tree")),
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
            id: id.to_string(),
            display_id: expectation.display_id.clone(),
        };
        state
            .write_last_result_for_record(
                root,
                git_backed.then_some("checked-tree"),
                expectation,
                &record,
            )
            .map(|_| ())
    }

    fn identity(id: &str) -> ExpectationIdentity {
        ExpectationIdentity {
            id: id.to_string(),
            display_id: id.to_string(),
        }
    }

    fn expectation(id: &str) -> crate::check::ResolvedExpectation {
        crate::check::ResolvedExpectation {
            kind: crate::check::ResolvedExpectationKind::Configured { id: id.to_string() },
            display_id: id.to_string(),
            to: ExpectationTo::Agent,
            rank: 0,
            question: "Does it pass?".to_string(),
            expected_answer: "yes".to_string(),
            question_context: String::new(),
            diff_from: DEFAULT_DIFF_FROM.to_string(),
            target: None,
            agent: AgentConfig::default(),
            cooldown: None,
            q_scope: Default::default(),
        }
    }

    fn git_project(name: &str) -> Result<PathBuf, String> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let root = std::env::temp_dir().join(format!("canon-{name}-{}-{unique}", process::id()));
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let output = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        Ok(root)
    }
}
