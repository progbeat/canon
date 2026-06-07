use crate::check::core::types::{CachedExpectation, SelectedExpectation};
use crate::check::run::selection::ExpectationIdentity;
use crate::config_types::CheckConfig;
use crate::fs_util::{
    ensure_dir_without_symlinks, for_each_nonempty_line, write_temp_file_then_replace,
};
use crate::git::resolve_git_path;
use crate::hash::full_scope;
use crate::logs::DiagnosticLogWriter;
use crate::state_paths::CANON_CACHE_DIR_GIT_PATH;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

pub(crate) fn apply_lazy_full_scope_reset(
    root: &Path,
    _config: &CheckConfig,
    evaluated_expectations: usize,
    cached: &[CachedExpectation],
    cache: &mut LazyFullScopeResetCache,
    diagnostic_log: &mut DiagnosticLogWriter,
) -> Result<(), String> {
    apply_lazy_full_scope_reset_for_cached(
        root,
        evaluated_expectations,
        cached,
        cache,
        diagnostic_log,
    )
}

pub(crate) fn apply_lazy_full_scope_reset_for_cached(
    root: &Path,
    evaluated_expectations: usize,
    cached: &[CachedExpectation],
    cache: &mut LazyFullScopeResetCache,
    diagnostic_log: &mut DiagnosticLogWriter,
) -> Result<(), String> {
    let reset =
        plan_lazy_full_scope_reset(evaluated_expectations, cached, cache, random_reset_seed())?;
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
    if let Err(error) = schedule_lazy_full_scope_resets(root, &reset.expectations, cache) {
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

pub(crate) fn activate_scheduled_lazy_full_scope_resets(
    root: &Path,
    cache: &mut LazyFullScopeResetCache,
) -> Result<(), String> {
    let ids = read_scheduled_lazy_full_scope_resets(root, cache)?;
    if !ids.is_empty() {
        // Activation is the scheduled reset taking effect for this invocation:
        // cache lookup treats active IDs as not reusable, and fresh
        // interrogation starts from full project scope. The active marker is
        // cleared only after that full-scope record is written.
        write_active_lazy_full_scope_reset_markers(root, &ids, cache)?;
        remove_scheduled_lazy_full_scope_resets(root, cache)?;
    }
    Ok(())
}

pub(crate) fn active_lazy_full_scope_reset_ids(
    root: &Path,
    identities: &[ExpectationIdentity],
    cache: &mut LazyFullScopeResetCache,
) -> Result<BTreeSet<String>, String> {
    cache.active_lazy_full_scope_reset_ids(root, identities)
}

pub(crate) fn clear_active_lazy_full_scope_reset(
    root: &Path,
    expectation: &SelectedExpectation,
    cache: &mut LazyFullScopeResetCache,
) -> Result<(), String> {
    clear_active_lazy_full_scope_reset_id(root, &expectation.id, cache)
}

pub(crate) fn clear_active_lazy_full_scope_reset_ids(
    root: &Path,
    ids: &BTreeSet<String>,
    cache: &mut LazyFullScopeResetCache,
) -> Result<(), String> {
    for id in ids {
        clear_active_lazy_full_scope_reset_id(root, id, cache)?;
    }
    Ok(())
}

fn clear_active_lazy_full_scope_reset_id(
    root: &Path,
    id: &str,
    cache: &mut LazyFullScopeResetCache,
) -> Result<(), String> {
    let path = cache.active_lazy_full_scope_reset_path(root, id)?;
    remove_active_lazy_full_scope_reset_at_path(&path)?;
    cache.clear_active_reset_marker_reads();
    Ok(())
}

pub(crate) struct LazyFullScopeResetPlan {
    pub(crate) evaluated_expectations: usize,
    pub(crate) candidate_count: usize,
    pub(crate) expectations: Vec<SelectedExpectation>,
}

#[derive(Default)]
pub(crate) struct LazyFullScopeResetCache {
    schedule_paths: BTreeMap<PathBuf, PathBuf>,
    active_paths: BTreeMap<(PathBuf, String), PathBuf>,
    scheduled_ids: BTreeMap<PathBuf, Vec<String>>,
    active_id_sets: BTreeMap<(PathBuf, Vec<String>), BTreeSet<String>>,
    candidates: BTreeMap<Vec<CachedExpectationFingerprint>, Vec<SelectedExpectation>>,
}

pub(crate) fn plan_lazy_full_scope_reset(
    evaluated_expectations: usize,
    cached: &[CachedExpectation],
    cache: &mut LazyFullScopeResetCache,
    seed: u64,
) -> Result<LazyFullScopeResetPlan, String> {
    let candidates = cache.lazy_full_scope_reset_candidates(cached);
    let reset_count = lazy_full_scope_reset_count(evaluated_expectations, seed, candidates.len());
    Ok(LazyFullScopeResetPlan {
        evaluated_expectations,
        candidate_count: candidates.len(),
        expectations: sample_reset_expectations(&candidates, reset_count, seed),
    })
}

impl LazyFullScopeResetCache {
    fn lazy_full_scope_reset_schedule_path(&mut self, root: &Path) -> Result<PathBuf, String> {
        let root = root.to_path_buf();
        if let Some(path) = self.schedule_paths.get(&root) {
            return Ok(path.clone());
        }
        let path = resolve_git_path(&root, "canon/lazy-full-scope-reset")?;
        self.schedule_paths.insert(root, path.clone());
        Ok(path)
    }

    fn active_lazy_full_scope_reset_path(
        &mut self,
        root: &Path,
        id: &str,
    ) -> Result<PathBuf, String> {
        let key = (root.to_path_buf(), id.to_string());
        if let Some(path) = self.active_paths.get(&key) {
            return Ok(path.clone());
        }
        let path = resolve_git_path(root, CANON_CACHE_DIR_GIT_PATH)?
            .join(id)
            .join("lazy-full-scope-reset");
        self.active_paths.insert(key, path.clone());
        Ok(path)
    }

    fn read_scheduled_lazy_full_scope_resets(
        &mut self,
        root: &Path,
    ) -> Result<Vec<String>, String> {
        let root_key = root.to_path_buf();
        if let Some(ids) = self.scheduled_ids.get(&root_key) {
            return Ok(ids.clone());
        }
        let path = self.lazy_full_scope_reset_schedule_path(root)?;
        let mut ids = Vec::new();
        for_each_nonempty_line(&path, |_, line| {
            ids.push(line);
            Ok(())
        })?;
        self.scheduled_ids.insert(root_key, ids.clone());
        Ok(ids)
    }

    fn remember_scheduled_lazy_full_scope_resets(&mut self, root: &Path, ids: Vec<String>) {
        self.scheduled_ids.insert(root.to_path_buf(), ids);
    }

    fn active_lazy_full_scope_reset_ids(
        &mut self,
        root: &Path,
        identities: &[ExpectationIdentity],
    ) -> Result<BTreeSet<String>, String> {
        let identity_ids = identities
            .iter()
            .map(|identity| identity.id.clone())
            .collect::<Vec<_>>();
        let key = (root.to_path_buf(), identity_ids.clone());
        if let Some(ids) = self.active_id_sets.get(&key) {
            return Ok(ids.clone());
        }
        let mut ids = BTreeSet::new();
        for id in identity_ids {
            if self.active_lazy_full_scope_reset_path(root, &id)?.exists() {
                ids.insert(id);
            }
        }
        self.active_id_sets.insert(key, ids.clone());
        Ok(ids)
    }

    fn clear_active_reset_marker_reads(&mut self) {
        self.active_id_sets.clear();
    }

    fn lazy_full_scope_reset_candidates(
        &mut self,
        cached: &[CachedExpectation],
    ) -> Vec<SelectedExpectation> {
        let fingerprint = cached_expectation_fingerprint(cached);
        if let Some(candidates) = self.candidates.get(&fingerprint) {
            return candidates.clone();
        }
        let full_scope = full_scope();
        let candidates = cached
            .iter()
            .filter(|cached| cached.record.passed() && cached.record.scope != full_scope)
            .map(|cached| cached.expectation.clone())
            .collect::<Vec<_>>();
        self.candidates.insert(fingerprint, candidates.clone());
        candidates
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CachedExpectationFingerprint {
    number: usize,
    id: String,
    display_id: String,
    question: String,
    answer: String,
    agent_models: Vec<String>,
    agent_thinking: String,
    agent_ignore: Vec<String>,
    agent_plugins: Vec<String>,
    cooldown_pass_seconds: Option<u64>,
    cooldown_fail_seconds: Option<u64>,
    record_passed: bool,
    record_scope: Vec<String>,
}

fn cached_expectation_fingerprint(
    cached: &[CachedExpectation],
) -> Vec<CachedExpectationFingerprint> {
    cached
        .iter()
        .map(|cached| {
            let cooldown = cached.expectation.cooldown;
            CachedExpectationFingerprint {
                number: cached.expectation.number,
                id: cached.expectation.id.clone(),
                display_id: cached.expectation.display_id.clone(),
                question: cached.expectation.q.clone(),
                answer: cached.expectation.a.clone(),
                agent_models: cached.expectation.agent.models.clone(),
                agent_thinking: cached.expectation.agent.thinking.clone(),
                agent_ignore: cached.expectation.agent.ignore.clone(),
                agent_plugins: cached.expectation.agent.plugins.clone(),
                cooldown_pass_seconds: cooldown.and_then(|cooldown| cooldown.pass_seconds),
                cooldown_fail_seconds: cooldown.and_then(|cooldown| cooldown.fail_seconds),
                record_passed: cached.record.passed(),
                record_scope: cached.record.scope.clone(),
            }
        })
        .collect()
}

fn write_active_lazy_full_scope_reset_markers(
    root: &Path,
    ids: &[String],
    cache: &mut LazyFullScopeResetCache,
) -> Result<(), String> {
    for id in ids {
        let path = cache.active_lazy_full_scope_reset_path(root, id)?;
        if let Some(parent) = path.parent() {
            ensure_dir_without_symlinks(parent)?;
        }
        let temp_path = lazy_full_scope_reset_schedule_temp_path(&path)?;
        write_temp_file_then_replace(&temp_path, &path, |file| {
            file.write_all(b"full\n")
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))
        })?;
    }
    cache.clear_active_reset_marker_reads();
    Ok(())
}

fn remove_active_lazy_full_scope_reset_at_path(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("failed to delete {}: {}", path.display(), err)),
    }
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
    cache: &mut LazyFullScopeResetCache,
) -> Result<(), String> {
    let path = cache.lazy_full_scope_reset_schedule_path(root)?;
    if expectations.is_empty() {
        remove_scheduled_lazy_full_scope_resets_at_path(&path)?;
        cache.remember_scheduled_lazy_full_scope_resets(root, Vec::new());
        return Ok(());
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
    })?;
    cache.remember_scheduled_lazy_full_scope_resets(
        root,
        expectations
            .iter()
            .map(|expectation| expectation.id.clone())
            .collect(),
    );
    Ok(())
}

fn read_scheduled_lazy_full_scope_resets(
    root: &Path,
    cache: &mut LazyFullScopeResetCache,
) -> Result<Vec<String>, String> {
    cache.read_scheduled_lazy_full_scope_resets(root)
}

fn remove_scheduled_lazy_full_scope_resets(
    root: &Path,
    cache: &mut LazyFullScopeResetCache,
) -> Result<(), String> {
    let path = cache.lazy_full_scope_reset_schedule_path(root)?;
    remove_scheduled_lazy_full_scope_resets_at_path(&path)?;
    cache.remember_scheduled_lazy_full_scope_resets(root, Vec::new());
    Ok(())
}

fn remove_scheduled_lazy_full_scope_resets_at_path(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("failed to delete {}: {}", path.display(), err)),
    }
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
