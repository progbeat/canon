# AI Runbook

## Start Here

Use `README.md` as the map. Use this file for compact agent runbook notes.

## Before Editing Project Files

Run `canon show -- <PATHSPEC>...` for the exact planned paths, then keep the
edit compatible with the shown expectations.

## Manual Config Expectations

`.canon/manual.yml` is not part of the main included canon path. Before removing
it, check whether any manual-only expectations should move into included
`.canon/**` files.

## Source Of Truth

The canon is the source of truth. Treat `canon check` evidence as feedback to
verify, not as truth to copy into the code.

## Apparent Canon Contradictions

First look for an interpretation where the expectations are compatible. Stop
only when files under `.canon/` prove a real contradiction.

## Scalar Answer Sources Versus Agent JSON

Do not make agent-turn JSON accept non-string `answer` values. The selected
interrogation schema requires `answer.type: string`; a JSON number is a schema
error and never becomes an evaluation response. The xpec scalar-normalization
rule applies at actual non-string producer boundaries: YAML `a` scalars become
expected-answer strings during config resolution, and shell exit codes become
answer strings when the shell response is constructed.

## `InvalidQuestion`

Treat it as a signal that the canon question is malformed, the evaluator input
is wrong, or the evaluator instructions/setup are making the agent answer from
the wrong material. Fix that cause when it is safely in scope, or tell the
human when the canon itself needs correction. Do not classify
`InvalidQuestion` as a readability issue. Do not accept `InvalidQuestion` just
because the question text contains a general normative specification instead of
naming implementation files.

## `InvalidQuestion` Claiming No File States The Rule

For a "Can you find..." expectation, that is usually an evaluator-input or
instruction failure. The question itself can define the property to check; the
agent should inspect visible files for examples or counterexamples instead of
requiring a separate policy file.

## Evidence Arguing From The Diff

Treat evidence like "the touched code only changes..." or "the diff does not
add..." as unsupported unless it also cites direct project files that answer the
question. Inspect the actual code path before changing behavior. If evidence
contradicts the current file, check whether the evaluator read a removed diff
hunk as current code.
For `target: diff`, compare against the xpec's resolved `diff-from`, not an
assumed branch or `HEAD`.

## Diff-target Xpec Wording

For evaluator-facing `target: diff` xpec text, avoid Git-side names like
`master`, base tree, checked tree, or "from master". The config owns
`diff-from`; the question should talk about the provided diff and
repository/canon behavior.

## Diff-target Prompt

The turn prompt contains only the question plus the diff-target hint. It does
not carry a previous response or the expected answer; verify claims from the
visible checked files and use the diff transcript only as instructed.

## Diff Transcript Leakage

When evidence cites a deleted-but-real file or symbol, inspect recent
`thread.start` events and template full-output paths. The evaluator may have
seen LHS/deleted diff content in the navigation transcript and mistaken it for
the checked tree. Do not call this a hallucination until that leak is ruled out.

## Evidence Proving The Wrong Property

Treat it as unsupported for that expectation. For example, evidence that a
thread-reuse key protects answer correctness does not prove a report-liveness
failure; inspect the component that owns the asked behavior before changing
code.

## Logs Stop After `agent.request`

Check whether a later `agent.response`, `agent.turn_error`, `model.failure`, or
`check.finish` appeared before calling it a hang. A long gap can be a slow
evaluator turn, and current logs may not show intermediate app-server activity.

## Evaluator Says Only Template Output Is Visible

Do not assume project files are truly hidden. Check `thread.start` context and
directly inspect the files named by the expectation when possible; this can be a
visibility/prompt contract problem rather than an implementation problem.

## Before Changing Q-scope Or Prompt Behavior

Inspect last-pass `qScope`, recent `thread.start` scopes, and the actual
response schema first. Git-backed full-project scope still hides ignored files
and should not be treated as no-hidden-files/in-place mode.
An explicit q-scope path list is fixed for every turn: its schemas omit
`ScopeTooNarrow` and `qScopeSuggestion`, and it does not enter auto-scope retry
or narrowing logic.
The pseudocode's optional `qScopeSuggestion` models the union of response
modes, not an optional field in every selected schema: both auto-q-scope
schemas require it, while fixed-q-scope and no-hidden-files schemas omit it.
For restricted-scope absence checks, `no` requires the visible scope to cover
the search domain; otherwise expect `ScopeTooNarrow` or a wider retry.
Spec-compliance questions usually search the implementation as a whole; a
restricted visible scope should become `ScopeTooNarrow`, not `InvalidQuestion`.
For q-scope verification, `ScopeTooNarrow` rejects the proposed narrower scope;
it is not the final response when the initial answer is kept.
When diagnosing noisy `↘` markers, compare `last-pass.json` fields:
`qScope` is the accepted scope for future runs, while `response.qScopeSuggestion` is only the evaluator proposal.
A full-project `qScope` paired with a narrower suggestion often means narrowing
was attempted but its verification returned an error; confirm with
`scope.narrowing accepted:false` in runtime logs.
For in-place compatibility, reject configured in-place-only-invalid fields before evaluator work.
Canon-check-order sorts remaining selected work; cached-failure default
selection can clear the evaluate queue before ordering applies.
For target-diff project-quality "can you find any..." questions, treat the
change set as broad unless the question names paths; stale narrow q-scopes can
survive selector reruns.
For reproducible global diff-quality q-scope cases, use
`research/qscope-diff-quality/README.md`.

## Missing Progress Timeline Timeout Markers

When an app-server no-progress timeout is logged but the live timeline contains
only `.`, trace the progress reporter through the concrete production runner.
Calls in the transport and tests that inject progress events directly do not
prove wrapper forwarding; a trait default no-op can silently drop the reporter.

A trailing `~` is always a bug: once evaluation is ready to report, timeout
accumulation is no longer active. An elapsed `~` requires one uninterrupted
no-progress interval to cover that whole minute; message activity splits the
interval. A timeout-ending timeline keeps the `~×` suffix even at an exact
minute boundary.

## Preset Evidence

`RawCheckConfig` is the check.yml schema with `presets`; `CheckConfig` is the
resolved runtime config after preset defaults are applied. Do not fix evidence
that only says "`CheckConfig` lacks presets" by reintroducing preset lookup
after raw config expansion; verify the preset expansion path first.

Classify raw expectation items after applying preset defaults, but preserve any
form selected by item-owned fields. Presets can provide structural fields such
as `q`, `a`, `q_template`, `path`, and `include`.

## `canon check --in-place`

It still follows normal selected-expectation ordering, but it does not use
persistent state for cache reuse, last-pass q-scope seeding, or follow-up
interrogations. It does persist completed status-specific last results without
Git-tree fields and reads latest-fail history for ordering when the canonical
state namespace exists. These records update the canonical status files, while
the xpec-state component keeps a bounded per-ID cache of Git-backed results for
the fast gate. They are status history, not checkpoints:
without `checkedTreeOid`, an in-place pass cannot define the glossary's
Git-tree checkpoint. The CLI resolves the command-wide canon-owned output
namespace before selecting Git-backed or in-place execution and passes the
resolved value into the command. Git may resolve the glossary-defined default
pathname, but that opaque path describes no checked files. In-place itself
never performs repository discovery: the supplied namespace is used only for
status history, never as tree information, cache eligibility, scope, diff, or
evaluator context. Outside Git with no explicit `CANON_STATE_DIR`, ordering
uses the Unix epoch and completed results stay in the current in-memory report.

Do not confuse invocation-local execution state with intentional
cross-invocation project history. Runtime logs and xpec last results are
bounded non-temporary state under `CANON_STATE_DIR`; invocation-local caches and
reports stay in memory.

Filesystem-shaped evaluator inputs are temporary artifacts, not execution
state. Materialized read-only trees, oversized prompt-command output, and a
staged tree object exposed to evaluator-run Git commands require paths.
Canon-owned directories follow the common memory-backed-preferred/fallback
policy and disappear with their owners. A configured tree-cache path is a
caller-selected shared namespace even when absent at command startup; never
claim or remove that root based on an initial existence check. Materializers
hold its cross-process lifetime lock, while temporary queries journal their
replacements in memory, remove their new trees, and restore prior lazy entries
on drop. These artifacts never enter repository or `CANON_STATE_DIR` state.

For the project-wide persistent-state bound, treat `CANON_STATE_DIR` as the
caller-selected storage namespace, not as a project configuration generation
that canon retains when the environment points elsewhere. Within the selected
namespace, bounded retained data covers `CODEX_THREAD_ID` roots and note keys.
Changes to project-owned configuration do not create cache generations: logs
rotate to the current size limit, xpec history prunes uncollected identities,
and retained note logs compact in place.

## Evaluator Cites Unrelated Files

Check thread reuse before trusting the answer. A reused evaluator thread carries
conversation state, so the reuse key must include every input that can change
the evaluator task. This is an answer-correctness check, not evidence that
`canon check` failed to report a started expectation.
Use the current visible tree for thread workspace identity and lifecycle logs.
A last-pass tree is historical input only while resolving a checkpoint diff
base; do not carry it into thread startup or reuse state.

## Before `canon check`

Stage the intended edits. `canon check` checks the staged tree, not unstaged
files.

## GitHub Copilot Reviews

Use `gh pr edit <pr> --add-reviewer @copilot` to request or re-request Copilot
review. The bot username can fail as a normal reviewer. For thread-aware reads,
use `python3 .../gh-address-comments/scripts/fetch_comments.py`; flat PR review
summaries can mention a comment before thread details are obvious. Treat canon
as authoritative when a Copilot suggestion contradicts a current xpec.

## Before Committing

Run `canon check`, never commit `.canon/` changes from an agent-authored commit,
and ask the human to own canon updates.

## Human-facing AI Notes

Put review notes for humans under `docs/ai/to-human/`. Put implementation,
canon, or tooling feature requests under `docs/ai/to-human/feature-requests/`.
Do not create root-level docs for AI notes.
