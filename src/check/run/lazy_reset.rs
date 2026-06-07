use crate::check::core::{CachedExpectation, CheckRecord, SelectedExpectation};
use crate::check::run::selection::ExpectationIdentity;
use crate::config_types::CheckConfig;
use crate::fs_util::{ensure_dir_without_symlinks, reject_symlink};
use crate::git::resolve_git_path;
use crate::hash::full_scope;
use crate::logs::DiagnosticLogWriter;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

// Matches `.canon/specs/canon-check-lazy-full-scope-reset.md`:
// stochastic_round(num_evaluated_expectations / 128).
const LAZY_FULL_SCOPE_RESET_EVALUATIONS_PER_SAMPLE: f64 = 128.0;
const LAZY_FULL_SCOPE_RESET_PENDING_GIT_PATH: &str = "canon/lazy-full-scope-reset";

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
        plan_lazy_full_scope_reset(evaluated_expectations, cached, cache, random_reset_seed()?)?;
    apply_lazy_full_scope_reset_plan(root, &reset, cache, diagnostic_log)
}

fn apply_lazy_full_scope_reset_plan(
    root: &Path,
    reset: &LazyFullScopeResetPlan,
    cache: &mut LazyFullScopeResetCache,
    diagnostic_log: &mut DiagnosticLogWriter,
) -> Result<(), String> {
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
    if let Err(error) = queue_lazy_full_scope_resets(root, &reset.expectations, cache) {
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

pub(crate) fn active_lazy_full_scope_reset_ids(
    root: &Path,
    identities: &[ExpectationIdentity],
    cache: &mut LazyFullScopeResetCache,
) -> Result<BTreeSet<String>, String> {
    cache.active_lazy_full_scope_reset_ids(root, identities)
}

pub(crate) fn clear_evaluated_lazy_full_scope_resets(
    root: &Path,
    active_ids: &BTreeSet<String>,
    records: &[CheckRecord],
    cache: &mut LazyFullScopeResetCache,
) -> Result<(), String> {
    let mut pending_ids = cache.read_pending_lazy_full_scope_reset_ids(root)?;
    let removed_ids = records
        .iter()
        .filter(|record| active_ids.contains(&record.id) && pending_ids.contains(&record.id))
        .map(|record| record.id.clone())
        .collect::<Vec<_>>();
    for id in &removed_ids {
        remove_lazy_full_scope_reset_marker(root, id, cache)?;
        pending_ids.remove(id);
    }
    cache.remember_pending_lazy_full_scope_reset_ids(root, pending_ids);
    Ok(())
}

pub(crate) struct LazyFullScopeResetPlan {
    pub(crate) evaluated_expectations: usize,
    pub(crate) candidate_count: usize,
    pub(crate) expectations: Vec<SelectedExpectation>,
}

#[derive(Default)]
pub(crate) struct LazyFullScopeResetCache {
    pending_paths: BTreeMap<PathBuf, PathBuf>,
    pending_ids: BTreeMap<PathBuf, BTreeSet<String>>,
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
    fn lazy_full_scope_reset_pending_path(&mut self, root: &Path) -> Result<PathBuf, String> {
        let root = root.to_path_buf();
        if let Some(path) = self.pending_paths.get(&root) {
            return Ok(path.clone());
        }
        let path = resolve_git_path(&root, LAZY_FULL_SCOPE_RESET_PENDING_GIT_PATH)?;
        self.pending_paths.insert(root, path.clone());
        Ok(path)
    }

    fn read_pending_lazy_full_scope_reset_ids(
        &mut self,
        root: &Path,
    ) -> Result<BTreeSet<String>, String> {
        let root_key = root.to_path_buf();
        if let Some(ids) = self.pending_ids.get(&root_key) {
            return Ok(ids.clone());
        }
        let mut ids = BTreeSet::new();
        for id in read_pending_lazy_full_scope_reset_marker_entries(
            &self.lazy_full_scope_reset_pending_path(root)?,
        )? {
            ids.insert(id);
        }
        self.pending_ids.insert(root_key, ids.clone());
        Ok(ids)
    }

    fn remember_pending_lazy_full_scope_reset_ids(&mut self, root: &Path, ids: BTreeSet<String>) {
        self.pending_ids.insert(root.to_path_buf(), ids);
    }

    fn active_lazy_full_scope_reset_ids(
        &mut self,
        root: &Path,
        identities: &[ExpectationIdentity],
    ) -> Result<BTreeSet<String>, String> {
        let pending_ids = self.read_pending_lazy_full_scope_reset_ids(root)?;
        Ok(identities
            .iter()
            .filter_map(|identity| {
                if pending_ids.contains(&identity.id) {
                    Some(identity.id.clone())
                } else {
                    None
                }
            })
            .collect())
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
                question: cached.expectation.question.clone(),
                answer: cached.expectation.expected_answer.clone(),
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

fn read_pending_lazy_full_scope_reset_marker_entries(path: &Path) -> Result<Vec<String>, String> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("failed to read {}: {}", path.display(), err)),
    };
    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
        let metadata = entry
            .metadata()
            .map_err(|err| format!("failed to inspect {}: {}", entry.path().display(), err))?;
        if !metadata.is_file() {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_string) else {
            return Err(format!(
                "pending lazy reset marker must be valid UTF-8: {}",
                entry.path().display()
            ));
        };
        ids.push(id);
    }
    Ok(ids)
}

fn remove_lazy_full_scope_reset_marker(
    root: &Path,
    id: &str,
    cache: &mut LazyFullScopeResetCache,
) -> Result<(), String> {
    let marker = lazy_full_scope_reset_marker_path(root, id, cache)?;
    match fs::remove_file(&marker) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(format!("failed to delete {}: {}", marker.display(), err)),
    }
    remove_empty_lazy_full_scope_reset_dir(root, cache)
}

fn remove_empty_lazy_full_scope_reset_dir(
    root: &Path,
    cache: &mut LazyFullScopeResetCache,
) -> Result<(), String> {
    let path = cache.lazy_full_scope_reset_pending_path(root)?;
    match fs::remove_dir(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(()),
        Err(err) => Err(format!("failed to delete {}: {}", path.display(), err)),
    }
}

pub(crate) fn lazy_full_scope_reset_count(
    evaluated_expectations: usize,
    seed: u64,
    candidate_count: usize,
) -> usize {
    let count = stochastic_round(
        evaluated_expectations as f64 / LAZY_FULL_SCOPE_RESET_EVALUATIONS_PER_SAMPLE,
        seed,
    );
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

pub(crate) fn queue_lazy_full_scope_resets(
    root: &Path,
    expectations: &[SelectedExpectation],
    cache: &mut LazyFullScopeResetCache,
) -> Result<(), String> {
    let mut pending_ids = cache.read_pending_lazy_full_scope_reset_ids(root)?;
    let new_ids = expectations
        .iter()
        .map(|expectation| expectation.id.clone())
        .filter(|id| !pending_ids.contains(id))
        .collect::<Vec<_>>();
    for id in &new_ids {
        write_lazy_full_scope_reset_marker(root, id, cache)?;
        pending_ids.insert(id.clone());
    }
    cache.remember_pending_lazy_full_scope_reset_ids(root, pending_ids);
    Ok(())
}

fn write_lazy_full_scope_reset_marker(
    root: &Path,
    id: &str,
    cache: &mut LazyFullScopeResetCache,
) -> Result<(), String> {
    let marker = lazy_full_scope_reset_marker_path(root, id, cache)?;
    if let Some(parent) = marker.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    reject_symlink(&marker)?;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
    {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(format!("failed to write {}: {}", marker.display(), err)),
    }
}

fn lazy_full_scope_reset_marker_path(
    root: &Path,
    id: &str,
    cache: &mut LazyFullScopeResetCache,
) -> Result<PathBuf, String> {
    Ok(cache.lazy_full_scope_reset_pending_path(root)?.join(id))
}

fn random_reset_seed() -> Result<u64, String> {
    getrandom::u64().map_err(|err| format!("failed to read lazy reset randomness: {}", err))
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
