# AI Runbook

## Start Here

Use `README.md` as the map. Use this file for compact agent runbook notes.

## Before Editing Project Files

Run `canon show -- <PATHSPEC>...` for the exact planned paths, then keep the
edit compatible with the shown expectations.

## Source Of Truth

The canon is the source of truth. Treat `canon check` evidence as feedback to
verify, not as truth to copy into the code.

## Apparent Canon Contradictions

First look for an interpretation where the expectations are compatible. Stop
only when files under `.canon/` prove a real contradiction.

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

## Diff-target Fast Path

For `target: diff`, do not force file reads before reusing the turn prompt's
previous valid response. Require reads only to prove a new answer or
`ScopeTooNarrow`.

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

Check whether a later `agent.response`, `model.failure`, or `check.finish`
appeared before calling it a hang. A long gap can be a slow evaluator turn, and
current logs may not show intermediate app-server activity.

## Evaluator Says Only Template Output Is Visible

Do not assume project files are truly hidden. Check `thread.start` context and
directly inspect the files named by the expectation when possible; this can be a
visibility/prompt contract problem rather than an implementation problem.

## Before Changing Q-scope Or Prompt Behavior

Inspect last-pass `qScope`, recent `thread.start` scopes, and the actual
response schema first. Git-backed full-project scope still hides ignored files
and should not be treated as no-hidden-files/in-place mode.
For restricted-scope absence checks, `no` requires the visible scope to cover
the search domain; otherwise expect `ScopeTooNarrow` or a wider retry.

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
persistent state for cache reuse, last-pass q-scope seeding, cooldowns,
follow-up interrogations, or last-result writes.

## Evaluator Cites Unrelated Files

Check thread reuse before trusting the answer. A reused evaluator thread carries
conversation state, so the reuse key must include every input that can change
the evaluator task. This is an answer-correctness check, not evidence that
`canon check` failed to report a started expectation.

## Before `canon check`

Stage the intended edits. `canon check` checks the staged tree, not unstaged
files.

## Before Committing

Run `canon check`, never commit `.canon/` changes from an agent-authored commit,
and ask the human to own canon updates.

## Human-facing AI Notes

Put review notes for humans under `docs/ai/to-human/`. Put implementation,
canon, or tooling feature requests under `docs/ai/to-human/feature-requests/`.
Do not create root-level docs for AI notes.
