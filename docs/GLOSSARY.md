# Glossary

This glossary covers the user-facing terms used by `canon` documentation and
CLI output.

## Ad-hoc question

A one-off question passed with `canon ask`. It is evaluated fresh instead
of being selected from the configured expectations.

## Cache

Stored last-result history that lets `canon` avoid asking the evaluator again
when a previous answer still applies to the current staged project state.

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

A time window during which a recent pass/fail last result can avoid being
re-proven for every small staged change. Cooldown is useful for broad review
expectations that are expensive to recheck on every commit.

## Cooldown result

A pass cached result derived from the latest pass/fail last result when that
result has a configured cooldown duration and its timestamp is still inside that
window.

## Evidence

The evaluator's explanation for an observed answer, usually citing the files or
code that support it.

## Expectation Instructions

The resolved `instructions` config value for an expectation, or empty text when
none is configured. Despite the config field name, this is expectation context
data, not an implementation-owned evaluator-agent prompt or instruction source.
Evaluator prompt and instruction templates live under `resources/prompts/` and
decide how to embed this data. Storing this human-authored canon data in config
does not make the config file an evaluator prompt source.

## Expectation ID

A 20-character base62 hash derived from the rendered expectation question, the
expected answer, and a deterministic hash of the resolved expectation
instructions.

## Expected answer

The answer written in an expectation's `a` field. `canon` compares this value
to the observed answer using exact string equality.

## Expectation

A question and expected answer that the project should satisfy. In
`.canon/check.yml`, a basic expectation has a `q` field and an `a` field. An
expectation may also provide an `instructions` config value.

## Evaluator thread

An ephemeral evaluator interaction context whose history is not persisted across
`canon check` invocations. Within one check run, an evaluator thread may only be
reused for an interrogation with the same evaluator model and the same rendered
developer instructions. Reuse may also require the same live thread-start
context inputs that affect evaluator tools, session root, or prompt-rendered
tree context.

## Evaluator Prompt Boundary

The implementation-owned evaluator prompt and instruction templates are the
resource files under `resources/prompts/`. Config values such as expectation
`instructions` are human-authored canon data passed into those templates, not
additional implementation-owned prompt templates or instruction sources.

## Generator item

A config entry that expands into additional expectations. Generator items
include `include` entries and path-generator entries. A path-generator entry
uses a path pattern, a question template, and an expected answer, and expands
matching checked files.

## Observed answer

The answer returned by the evaluator for an expectation or ad-hoc question.
For configured expectations, `canon` compares the observed answer to the
expected answer.

## Pre-commit hook

The Git hook installed by `canon pre-commit install`. It runs `canon gate` before a
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

A pass or fail last result for an expectation whose stored `visibleTreeOid`
matches the current `visibleTreeOid` for that result's visible scope.

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
formed from the interrogation q-scope plus configured ignore exclusions. A fresh
interrogation starts from the `qScope` in the expectation's `last-pass.json`, or
full project scope when no last pass is available. Configured ignore patterns
are normalized as project-relative pathspec items, converted to excluding
pathspec items, and applied last.

## Visible tree

The scoped tree induced by a visible scope.

## Implementation Map

`config_types::Expectation` and `check::core::SelectedExpectation` carry
expectation questions, expected answers, the resolved `instructions` config
text, and target metadata from config loading into check runs.

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

`check::interrogation::policy::initial_q_scope_for_fresh_interrogation` forms
the base q-scope from the expectation's `last-pass.json`, or full project scope
when no last pass result exists. `xpec_state::last_result` reads and writes the
status-specific last-result files. `staged::worktree::StagedWorktreeView::materialize_visible_scope`
then applies the visible scope before creating the evaluator working tree.

`check::core::EvaluatorResponseJson` parses evaluator evidence and the required
`qScopeSuggestion` value. `check::interrogation::policy` treats suggestions as
unverified claims until an independent verification turn accepts them.
`xpec_state` persists status-specific last-result files with `qScope`,
`visibleScope`, and status-dependent tree OIDs. It reads last-pass `qScope`
when seeding future interrogations and reads pass/fail last results when
checking same-tree cached results.

`evaluator::protocol::prompt` renders the prompt templates stored under
`resources/prompts/` with MiniJinja. The developer-instructions template is
`resources/prompts/evaluator_developer_instructions.txt`; the renderer registers
the `json`, `shq`, `shargs`, and `sh` filters and runs `sh` blocks from the
repository root.

`check::interrogation::ask_with_reused_thread` enforces evaluator-thread reuse.
Its lookup key includes the evaluator model and the runtime inputs that render
the developer-instructions transcript for the current prompt template. It also
splits on live thread-start context outside that rendered string, such as
plugins and the visible-scope/session-tree inputs that determine the evaluator
working tree.
