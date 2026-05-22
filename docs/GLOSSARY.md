# Glossary

This glossary covers the user-facing terms used by `canon` documentation and
CLI output.

## Ad-hoc question

A one-off question passed with `canon check -q`. It is evaluated fresh instead
of being selected from the configured expectations.

## Cache

Stored check history that lets `canon` avoid asking the evaluator again when a
previous answer still applies to the current staged project state.

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
asks the evaluator when a reusable cached result is not available and records
the resulting answer, evidence, and scope.

## `canon gate`

The command used by the pre-commit hook. It checks the staged project state
against existing canon history and fails quickly when a commit needs a fresh
`canon check` or contains a new regression.

## Scope

The smallest set of repository paths that is sufficient for the evaluator to
answer a question correctly. Full project scope is written as `.`.

## Scope narrowing

The process of checking whether an expectation can be answered from a smaller
scope than the full project. Narrower scopes make cached results more reusable
when unrelated files change.

## Staged snapshot

The temporary Git-tracked project state that `canon check` evaluates. It comes
from the Git index, so unstaged and untracked working tree files are not part of
the snapshot.
