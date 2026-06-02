use crate::check::selection::ExpectationIdentity;
use crate::check::types::{CachedExpectation, SelectedExpectation};
use crate::config_types::CheckConfig;
use crate::fs_util::{
    ensure_dir_without_symlinks, for_each_nonempty_line, write_temp_file_then_replace,
};
use crate::git::resolve_git_path;
use crate::hash::full_scope;
use crate::logs::DiagnosticLogWriter;
use crate::state_paths::CANON_CACHE_DIR_GIT_PATH;
use serde_json::json;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

pub(crate) fn apply_lazy_full_scope_reset(
    root: &Path,
    _config: &CheckConfig,
    evaluated_expectations: usize,
    cached: &[CachedExpectation],
    diagnostic_log: &mut DiagnosticLogWriter,
) -> Result<(), String> {
    apply_lazy_full_scope_reset_for_cached(root, evaluated_expectations, cached, diagnostic_log)
}

pub(crate) fn apply_lazy_full_scope_reset_for_cached(
    root: &Path,
    evaluated_expectations: usize,
    cached: &[CachedExpectation],
    diagnostic_log: &mut DiagnosticLogWriter,
) -> Result<(), String> {
    let reset =
        plan_lazy_full_scope_reset(root, evaluated_expectations, cached, random_reset_seed())?;
    diagnostic_log
        .write_event(
            "info",
            "lazy_full_scope_reset",
            &[
                ("evaluated", json!(reset.evaluated_expectations)),
                ("candidates", json!(reset.candidate_count)),
                ("reset", json!(reset.expectations.len())),
                (
                    "ids",
                    json!(reset
                        .expectations
                        .iter()
                        .map(|expectation| expectation.id.clone())
                        .collect::<Vec<_>>()),
                ),
            ],
        )
        .map_err(|err| err.to_string())?;
    if let Err(error) = schedule_lazy_full_scope_resets(root, &reset.expectations) {
        diagnostic_log
            .write_event(
                "error",
                "lazy_full_scope_reset.error",
                &[("message", json!(error.clone()))],
            )
            .map_err(|err| err.to_string())?;
        return Err(error);
    }
    Ok(())
}

pub(crate) fn activate_scheduled_lazy_full_scope_resets(root: &Path) -> Result<(), String> {
    let ids = read_scheduled_lazy_full_scope_resets(root)?;
    if !ids.is_empty() {
        // Activation is the scheduled reset taking effect for this invocation:
        // cache lookup treats active IDs as not reusable, and fresh
        // interrogation starts from full project scope. The active marker is
        // cleared only after that full-scope record is written.
        write_active_lazy_full_scope_reset_markers(root, &ids)?;
        remove_scheduled_lazy_full_scope_resets(root)?;
    }
    Ok(())
}

pub(crate) fn active_lazy_full_scope_reset_ids(
    root: &Path,
    identities: &[ExpectationIdentity],
) -> Result<BTreeSet<String>, String> {
    let mut ids = BTreeSet::new();
    for identity in identities {
        if active_lazy_full_scope_reset_path(root, &identity.id)?.exists() {
            ids.insert(identity.id.clone());
        }
    }
    Ok(ids)
}

pub(crate) fn clear_active_lazy_full_scope_reset(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<(), String> {
    clear_active_lazy_full_scope_reset_id(root, &expectation.id)
}

pub(crate) fn clear_active_lazy_full_scope_reset_ids(
    root: &Path,
    ids: &BTreeSet<String>,
) -> Result<(), String> {
    for id in ids {
        clear_active_lazy_full_scope_reset_id(root, id)?;
    }
    Ok(())
}

fn clear_active_lazy_full_scope_reset_id(root: &Path, id: &str) -> Result<(), String> {
    remove_active_lazy_full_scope_reset_at_path(&active_lazy_full_scope_reset_path(root, id)?)
}

pub(crate) struct LazyFullScopeResetPlan {
    pub(crate) evaluated_expectations: usize,
    pub(crate) candidate_count: usize,
    pub(crate) expectations: Vec<SelectedExpectation>,
}

#[derive(Clone)]
struct CachedPassingExpectation {
    expectation: SelectedExpectation,
    scope: Vec<String>,
}

pub(crate) fn plan_lazy_full_scope_reset(
    _root: &Path,
    evaluated_expectations: usize,
    cached: &[CachedExpectation],
    seed: u64,
) -> Result<LazyFullScopeResetPlan, String> {
    let cached_passes = cached_passing_expectations_with_scope(cached);
    let candidates = lazy_full_scope_reset_candidates(&cached_passes);
    let reset_count = lazy_full_scope_reset_count(evaluated_expectations, seed, candidates.len());
    Ok(LazyFullScopeResetPlan {
        evaluated_expectations,
        candidate_count: candidates.len(),
        expectations: sample_reset_expectations(&candidates, reset_count, seed),
    })
}

fn write_active_lazy_full_scope_reset_markers(root: &Path, ids: &[String]) -> Result<(), String> {
    for id in ids {
        let path = active_lazy_full_scope_reset_path(root, id)?;
        if let Some(parent) = path.parent() {
            ensure_dir_without_symlinks(parent)?;
        }
        let temp_path = lazy_full_scope_reset_schedule_temp_path(&path)?;
        write_temp_file_then_replace(&temp_path, &path, |file| {
            file.write_all(b"full\n")
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))
        })?;
    }
    Ok(())
}

fn active_lazy_full_scope_reset_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    Ok(resolve_git_path(root, CANON_CACHE_DIR_GIT_PATH)?
        .join(id)
        .join("lazy-full-scope-reset"))
}

fn remove_active_lazy_full_scope_reset_at_path(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("failed to delete {}: {}", path.display(), err)),
    }
}

fn cached_passing_expectations_with_scope(
    cached: &[CachedExpectation],
) -> Vec<CachedPassingExpectation> {
    let mut scoped = Vec::new();
    for cached in cached {
        if !cached.record.passed() {
            continue;
        }
        scoped.push(CachedPassingExpectation {
            expectation: cached.expectation.clone(),
            scope: cached.record.scope.clone(),
        });
    }
    scoped
}

fn lazy_full_scope_reset_candidates(
    cached: &[CachedPassingExpectation],
) -> Vec<SelectedExpectation> {
    cached
        .iter()
        .filter(|expectation| expectation.scope != full_scope())
        .map(|expectation| expectation.expectation.clone())
        .collect()
}

pub(crate) fn lazy_full_scope_reset_count(
    evaluated_expectations: usize,
    seed: u64,
    candidate_count: usize,
) -> usize {
    let count = stochastic_round(evaluated_expectations as f64 / 128.0, seed);
    std::cmp::min(count, candidate_count)
}

fn stochastic_round(value: f64, seed: u64) -> usize {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    if value >= usize::MAX as f64 {
        return usize::MAX;
    }
    let floor = value.floor();
    let mut count = floor as usize;
    let probability = value - floor;
    if probability > 0.0 {
        let mut rng = ResetRng::new(seed);
        let draw = (rng.next_u64() as f64) / (u64::MAX as f64 + 1.0);
        if draw < probability {
            count += 1;
        }
    }
    count
}

pub(crate) fn sample_reset_expectations(
    candidates: &[SelectedExpectation],
    count: usize,
    seed: u64,
) -> Vec<SelectedExpectation> {
    if count == 0 {
        return Vec::new();
    }
    let mut sampled = candidates.to_vec();
    let mut rng = ResetRng::new(seed ^ 0x9e37_79b9_7f4a_7c15);
    for index in 0..sampled.len() {
        let remaining = sampled.len() - index;
        let swap = index + rng.next_bounded(remaining as u64) as usize;
        sampled.swap(index, swap);
    }
    sampled.truncate(count);
    sampled
}

pub(crate) fn schedule_lazy_full_scope_resets(
    root: &Path,
    expectations: &[SelectedExpectation],
) -> Result<(), String> {
    let path = lazy_full_scope_reset_schedule_path(root)?;
    if expectations.is_empty() {
        return remove_scheduled_lazy_full_scope_resets_at_path(&path);
    }
    let temp_path = lazy_full_scope_reset_schedule_temp_path(&path)?;
    write_temp_file_then_replace(&temp_path, &path, |file| {
        for expectation in expectations {
            file.write_all(expectation.id.as_bytes())
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
            file.write_all(b"\n")
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
        }
        Ok(())
    })
}

fn read_scheduled_lazy_full_scope_resets(root: &Path) -> Result<Vec<String>, String> {
    let path = lazy_full_scope_reset_schedule_path(root)?;
    let mut ids = Vec::new();
    for_each_nonempty_line(&path, |_, line| {
        ids.push(line);
        Ok(())
    })?;
    Ok(ids)
}

fn remove_scheduled_lazy_full_scope_resets(root: &Path) -> Result<(), String> {
    let path = lazy_full_scope_reset_schedule_path(root)?;
    remove_scheduled_lazy_full_scope_resets_at_path(&path)
}

fn remove_scheduled_lazy_full_scope_resets_at_path(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("failed to delete {}: {}", path.display(), err)),
    }
}

fn lazy_full_scope_reset_schedule_path(root: &Path) -> Result<std::path::PathBuf, String> {
    resolve_git_path(root, "canon/lazy-full-scope-reset")
}

fn lazy_full_scope_reset_schedule_temp_path(path: &Path) -> Result<PathBuf, String> {
    let file_name = path.file_name().ok_or_else(|| {
        format!(
            "lazy reset schedule path has no file name: {}",
            path.display()
        )
    })?;
    let mut temp_name = file_name.to_os_string();
    temp_name.push(format!(
        ".tmp.{}.{}",
        std::process::id(),
        random_reset_seed()
    ));
    Ok(path.with_file_name(temp_name))
}

fn random_reset_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0)
}

struct ResetRng {
    state: u64,
}

impl ResetRng {
    fn new(seed: u64) -> ResetRng {
        ResetRng { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn next_bounded(&mut self, upper: u64) -> u64 {
        self.next_bounded_u128(upper as u128) as u64
    }

    fn next_bounded_u128(&mut self, upper: u128) -> u128 {
        if upper <= 1 {
            return 0;
        }
        let threshold = upper.wrapping_neg() % upper;
        loop {
            let value = self.next_u128();
            if value >= threshold {
                return value % upper;
            }
        }
    }

    fn next_u128(&mut self) -> u128 {
        ((self.next_u64() as u128) << 64) | self.next_u64() as u128
    }
}
