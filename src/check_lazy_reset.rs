use crate::check_selection::{parse_cooldown, ExpectationIdentity};
use crate::check_types::{CheckRecord, ObservedAnswerState, SelectedExpectation};
use crate::config_types::{AgentConfig, CheckConfig};
use crate::fs_util::{for_each_nonempty_line, write_temp_file_then_replace};
use crate::git::resolve_git_path;
use crate::hash::full_scope;
use crate::history::{read_history_records_from_path, HistoryCache};
use crate::history_compaction::compact_history_temp_path;
use crate::history_reuse::latest_history_scope_with_cache;
use crate::logging::render_check_log_record;
use crate::logging::DiagnosticLogWriter;
use crate::scope::sanitize_scope_for_hash;
use serde_json::json;
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;

pub(crate) fn apply_lazy_full_scope_reset(
    root: &Path,
    config: &CheckConfig,
    evaluated_expectations: usize,
    non_selected: &[SelectedExpectation],
    diagnostic_log: &mut DiagnosticLogWriter,
) -> Result<(), String> {
    let reset = plan_lazy_full_scope_reset(
        root,
        &config.agent,
        evaluated_expectations,
        non_selected,
        random_reset_seed(),
    )?;
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

pub(crate) fn apply_scheduled_lazy_full_scope_resets(
    root: &Path,
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
) -> Result<usize, String> {
    let ids = read_scheduled_lazy_full_scope_resets(root)?;
    if ids.is_empty() {
        return Ok(0);
    }
    let expectations = scheduled_reset_expectations(config, identities, &ids)?;
    set_non_selected_expectation_scopes_to_full(root, &expectations)?;
    remove_scheduled_lazy_full_scope_resets(root)?;
    Ok(expectations.len())
}

pub(crate) struct LazyFullScopeResetPlan {
    pub(crate) evaluated_expectations: usize,
    pub(crate) candidate_count: usize,
    pub(crate) expectations: Vec<SelectedExpectation>,
}

#[derive(Clone)]
struct ScopedNonSelectedExpectation {
    expectation: SelectedExpectation,
    scope: Vec<String>,
}

pub(crate) fn plan_lazy_full_scope_reset(
    root: &Path,
    agent: &AgentConfig,
    evaluated_expectations: usize,
    non_selected: &[SelectedExpectation],
    seed: u64,
) -> Result<LazyFullScopeResetPlan, String> {
    let scoped_non_selected =
        non_selected_expectations_with_current_scope(root, agent, non_selected)?;
    // Spec candidates are only non-selected expectations whose current reusable
    // scope seed is narrower than full scope.
    let candidates = lazy_full_scope_reset_candidates(&scoped_non_selected);
    let reset_count = lazy_full_scope_reset_count(evaluated_expectations, seed, candidates.len());
    Ok(LazyFullScopeResetPlan {
        evaluated_expectations,
        candidate_count: candidates.len(),
        expectations: sample_reset_expectations(&candidates, reset_count, seed),
    })
}

fn non_selected_expectations_with_current_scope(
    root: &Path,
    agent: &AgentConfig,
    non_selected: &[SelectedExpectation],
) -> Result<Vec<ScopedNonSelectedExpectation>, String> {
    // `SelectedExpectation` does not store mutable answer-history scope. The
    // policy's `e.scope` is Canon's latest reusable history scope, or full
    // scope when no reusable answer exists.
    let mut history_cache = HistoryCache::new();
    let mut scoped = Vec::new();
    for expectation in non_selected {
        let scope = latest_history_scope_with_cache(root, agent, expectation, &mut history_cache)?
            .unwrap_or_else(full_scope);
        scoped.push(ScopedNonSelectedExpectation {
            expectation: expectation.clone(),
            scope,
        });
    }
    Ok(scoped)
}

fn lazy_full_scope_reset_candidates(
    non_selected: &[ScopedNonSelectedExpectation],
) -> Vec<SelectedExpectation> {
    non_selected
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

pub(crate) fn set_non_selected_expectation_scopes_to_full(
    root: &Path,
    expectations: &[SelectedExpectation],
) -> Result<(), String> {
    let mut history_cache = HistoryCache::new();
    for expectation in expectations {
        set_expectation_scope_to_full_for_next_check(root, expectation, &mut history_cache)?;
    }
    Ok(())
}

pub(crate) fn schedule_lazy_full_scope_resets(
    root: &Path,
    expectations: &[SelectedExpectation],
) -> Result<(), String> {
    let path = lazy_full_scope_reset_schedule_path(root)?;
    if expectations.is_empty() {
        return remove_scheduled_lazy_full_scope_resets_at_path(&path);
    }
    let temp_path = compact_history_temp_path(&path)?;
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

fn scheduled_reset_expectations(
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    ids: &[String],
) -> Result<Vec<SelectedExpectation>, String> {
    let mut expectations = Vec::new();
    for id in ids {
        let Some(index) = identities.iter().position(|identity| &identity.id == id) else {
            continue;
        };
        let identity = &identities[index];
        let expectation = &config.expectations[index];
        expectations.push(SelectedExpectation {
            number: index + 1,
            id: identity.id.clone(),
            display_id: identity.display_id.clone(),
            q: expectation.q.clone(),
            a: expectation.a.clone(),
            cooldown: expectation
                .cooldown
                .as_deref()
                .map(parse_cooldown)
                .transpose()?,
            thinking: expectation.thinking.clone(),
        });
    }
    Ok(expectations)
}

fn set_expectation_scope_to_full_for_next_check(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<(), String> {
    // `SelectedExpectation` does not store a mutable scope field. Canon's
    // set_scope(expectation, ["."]) operation is persisted by removing newer
    // narrowed answer records, which exposes an older full-scope seed or no
    // reusable seed for the next `canon check`.
    let path = history_cache.path(root, expectation)?;
    if !path.exists() {
        return Ok(());
    }
    let mut records = read_history_records_from_path(&path)?;
    let mut removed_narrowed = false;
    while let Some((index, scope)) = latest_reusable_record_scope(&records, expectation) {
        if scope == full_scope() {
            break;
        }
        // Lazy reset changes only the next interrogation scope seed. Removing
        // newer narrowed answer records exposes an older full-scope seed, or no
        // seed at all, without minting a fake full-scope cache hit. Every
        // remaining history record keeps the scopeTreeOid that belongs to its
        // own stored scope.
        records.remove(index);
        removed_narrowed = true;
    }
    if !removed_narrowed {
        return Ok(());
    }

    let temp_path = compact_history_temp_path(&path)?;
    write_temp_file_then_replace(&temp_path, &path, |file| {
        for record in records {
            let line = render_check_log_record(&record).map_err(|err| err.to_string())?;
            file.write_all(line.as_bytes())
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
        }
        Ok(())
    })?;
    history_cache.records.remove(&path);
    history_cache.reusable_records.clear();
    Ok(())
}

fn latest_reusable_record_scope(
    records: &[CheckRecord],
    expectation: &SelectedExpectation,
) -> Option<(usize, Vec<String>)> {
    records
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, record)| {
            reusable_record_scope(record, expectation).map(|scope| (index, scope))
        })
}

fn reusable_record_scope(
    record: &CheckRecord,
    expectation: &SelectedExpectation,
) -> Option<Vec<String>> {
    if !ObservedAnswerState::from_expected_and_observed(&expectation.a, &record.observed)
        .is_reusable_history()
    {
        return None;
    }
    sanitize_scope_for_hash(&record.scope).ok()
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
