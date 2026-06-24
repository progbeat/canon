# AI FAQ

## Where do I start?

Use `README.md` as the map. Use this file for compact agent runbook notes.

## What must happen before editing project files?

Run `canon show -- <PATHSPEC>...` for the exact planned paths, then keep the
edit compatible with the shown expectations.

## What is the source of truth?

The canon is the source of truth. Treat `canon check` evidence as feedback to
verify, not as truth to copy into the code.

## What if canon expectations seem contradictory?

First look for an interpretation where the expectations are compatible. Stop
only when files under `.canon/` prove a real contradiction.

## What if an evaluator result is `InvalidQuestion`?

Treat it as a signal that the canon question is malformed, the evaluator input
is wrong, or the evaluator instructions/setup are making the agent answer from
the wrong material. Fix that cause when it is safely in scope, or tell the
human when the canon itself needs correction. Do not accept `InvalidQuestion`
just because the question text contains a general normative specification
instead of naming implementation files.

## What if `InvalidQuestion` says no file states the rule?

For a "Can you find..." expectation, that is usually an evaluator-input or
instruction failure. The question itself can define the property to check; the
agent should inspect visible files for examples or counterexamples instead of
requiring a separate policy file.

## What if evidence argues from the diff?

Treat evidence like "the touched code only changes..." or "the diff does not
add..." as unsupported unless it also cites direct project files that answer the
question. Inspect the actual code path before changing behavior. If evidence
contradicts the current file, check whether the evaluator read a removed diff
hunk as current code.

## What if evidence proves a different property than the question asks?

Treat it as unsupported for that expectation. For example, evidence that a
thread-reuse key protects answer correctness does not prove a report-liveness
failure; inspect the component that owns the asked behavior before changing
code.

## What if logs stop after `agent.request`?

Check whether a later `agent.response`, `model.failure`, or `check.finish`
appeared before calling it a hang. A long gap can be a slow evaluator turn, and
current logs may not show intermediate app-server activity.

## What if the evaluator says only template output is visible?

Do not assume project files are truly hidden. Check `thread.start` context and
directly inspect the files named by the expectation when possible; this can be a
visibility/prompt contract problem rather than an implementation problem.

## What should I check before changing q-scope or prompt behavior?

Inspect stored q-scopes, recent `thread.start` scopes, and the actual response
schema first. Git-backed full-project scope still hides ignored files and should
not be treated as no-hidden-files/in-place mode.

## What is special about `canon check --in-place`?

It still follows normal selected-expectation ordering, but it does not use
persistent state for cache reuse, stored q-scopes, cooldowns, follow-up
interrogations, or last-result writes.

## What if an evaluator cites unrelated files for a question?

Check thread reuse before trusting the answer. A reused evaluator thread carries
conversation state, so the reuse key must include every input that can change
the evaluator task. This is an answer-correctness check, not evidence that
`canon check` failed to report a started expectation.

## What must happen before `canon check`?

Stage the intended edits. `canon check` checks the staged tree, not unstaged
files.

## What must happen before committing?

Run `canon check`, never commit `.canon/` changes from an agent-authored commit,
and ask the human to own canon updates.

## Where should human-facing AI notes go?

Put review notes for humans under `docs/ai/to-human/`. Put implementation,
canon, or tooling feature requests under `docs/ai/to-human/feature-requests/`.
Do not create root-level docs for AI notes.
