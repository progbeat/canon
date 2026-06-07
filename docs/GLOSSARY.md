# Glossary

This glossary covers the user-facing terms used by `canon` documentation and
CLI output.

## Ad-hoc question

A one-off question passed with `canon check -q`. It is evaluated fresh instead
of being selected from the configured expectations.

## Cache

Stored check history that lets `canon` avoid asking the evaluator again when a
previous answer still applies to the current staged project state.

## Cached result

The reusable result for an expectation in the current Git state. It is the
expectation's same-tree result when one exists; otherwise it is the
expectation's cooldown result, when one exists.

## Canon

The set of expectations that describes what must stay true for a project. In a
typical project, the canon lives in `.canon/check.yml`.

## Canon policy change

A change under `.canon/**` that changes what the project checks. Keep canon
policy changes separate from implementation changes so `canon gate` can tell
whether code changed or the rules changed.

## Config

The YAML file that defines evaluator settings and expectations. By default,
`canon` reads `.canon/check.yml`; `canon check --config <path>` and
`canon check -c <path>` read another config file.

## Cooldown

A time window during which a recent answer-history result can avoid being
re-proven for every small staged change. Cooldown is useful for broad review
expectations that are expensive to recheck on every commit.

## Cooldown result

A pass cached result derived from the latest answer-history record when that
record's current pass or fail result has a configured cooldown duration and its
timestamp is still inside that window.

## Evidence

The evaluator's explanation for an observed answer, usually citing the files or
code that support it.

## Expected answer

The answer written in an expectation's `a` field. `canon` compares this value
to the observed answer using exact string equality.

## Expectation

A question and expected answer that the project should satisfy. In
`.canon/check.yml`, a basic expectation has a `q` field and an `a` field.

## Generator item

A config entry that expands matching Markdown specs into additional
expectations. A generator item uses a path pattern, a question template, and an
expected answer.

## Observed answer

The answer returned by the evaluator for an expectation or ad-hoc question.
For configured expectations, `canon` compares the observed answer to the
expected answer.

## Pre-commit hook

The Git hook installed by `canon hook install`. It runs `canon gate` before a
commit and blocks staged changes that are not safe under the current canon
history.

## `canon check`

The command that evaluates expectations against the staged project state. It
collects configured expectations, computes cached results, and selects only the
expectations that require evaluator work. With no selectors, cached failures are
reported without fresh evaluation; if every cached result is a pass, uncached
expectations are evaluated.

## `canon gate`

The command used by the pre-commit hook. It checks the staged project state
against existing canon history and fails quickly when staged changes regress a
cached pass. Missing cached results are non-blocking; run `canon check` when
fresh confirmation is needed.

## Scope

A Git pathspec list that defines a subset of tracked repository files. Full
project scope is written as `.`.

## Q-scope

A question scope: a scope complete for a question. If files outside the q-scope
change while files inside it stay the same, the correct answer to the question
should not change.

## Q-scope suggestion

An evaluator-provided scope claiming to be narrow enough to answer the current
question. It may or may not be a valid q-scope. `canon check` only attempts to
verify a schema-valid suggestion when its induced visible tree has at least 25%
fewer files than the current visible tree.

## Scope narrowing

The runtime process for trying to store a narrower q-scope. When an evaluator
returns an answer with a q-scope suggestion, `canon check` may run an independent
interrogation under that suggested scope. The suggestion is stored only when
that verification produces a valid evaluator response with an answer.

## Same-tree result

The latest answer-history record for an expectation whose stored
`visibleTreeOid` matches the current `visibleTreeOid` for that record's scope.

## Selected expectations

The expectations that require evaluator work in the current `canon check` run.
Explicit selectors select matching expectations directly; the default
no-selector check subtracts cached passing expectations and evaluates none while
cached failures are present.

## Staged snapshot

The temporary Git-tracked project state that `canon check` evaluates. It comes
from the Git index, so unstaged and untracked working tree files are not part of
the snapshot.

## `visibleTreeOid`

The Git-compatible object ID of the tracked tree entries visible to the
evaluator after the visible scope is applied.

## Visible scope

The scope applied to a staged tracked tree for an evaluator interrogation. It is
formed from the latest verified q-scope for the expectation, or full project
scope when no verified q-scope is stored. Configured ignore patterns are
normalized as project-relative pathspec items, converted to excluding pathspec
items, and applied last.

## Visible tree

The scoped tree induced by a visible scope.

## Implementation Map

`config_types::Expectation` and `check::core::SelectedExpectation` carry
expectation questions and expected answers from config loading into check runs.

`scope` keeps scopes as Git pathspec lists: it normalizes repository paths,
forms visible scopes by appending configured ignore patterns as excluding
pathspec items, and matches tracked paths against those pathspec lists.

`git::visible_tree_oid` implements scoped tree and visible tree identity. It
collects the tracked entries induced by a scope, then computes the
repository-native Git tree object ID for those entries. When a scoped directory
already has a Git tree object, the implementation can reuse that object ID;
otherwise it serializes and hashes a synthetic tree object with the
repository's object hash algorithm. `staged::worktree` uses that same OID when
materializing evaluator-visible trees.

`check::interrogation::state::initial_visible_scope_for_expectation` forms the
base q-scope from the latest verified q-scope, or full project scope when none
is available. `staged::worktree::StagedWorktreeView::materialize_visible_scope`
then applies the visible scope before creating the evaluator working tree.

`check::core::EvaluatorResponseJson` parses evaluator evidence and the required
`qScopeSuggestion` value. `check::interrogation::policy` treats suggestions as
unverified claims until an independent verification turn accepts them.
`history` persists answer records with `visibleScope` and `visibleTreeOid`;
`history::reuse` reads reusable answer history when seeding future q-scopes or
same-tree cached results.

`check::interrogation::ask_with_reused_thread` enforces evaluator-thread reuse.
Its lookup key begins with evaluator model and `visibleTreeOid`, so a different
model or visible tree cannot reuse an existing evaluator thread.
