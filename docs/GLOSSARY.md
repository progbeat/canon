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

The reusable result for an expectation in the current Git state. It is the newer
of the expectation's same-tree result and cooldown result, when either exists.

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

A time window during which a recent passing result can remain valid without
being re-proven for every small staged change. Cooldown is useful for broad
review expectations that are expensive to recheck on every commit.

## Cooldown result

A passing answer-history record that is still inside the expectation's
configured cooldown window.

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
against existing canon history and fails quickly when a commit needs a fresh
`canon check` or contains a new regression.

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
verify a valid suggestion when its induced visible tree has at least 25% fewer
files than the current visible tree.

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
evaluator after enforced scope and ignore rules are applied.

## Visible scope

The scope applied to a staged tracked tree for an evaluator interrogation. It is
the latest stored q-scope for the expectation, or full project scope when no
q-scope is stored, with configured ignore patterns applied last.

## Visible tree

The scoped tree induced by a visible scope.
