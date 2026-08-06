# Glossary

This glossary covers the user-facing terms used by `canon` documentation and
CLI output.

## Ad-hoc question

A one-off question passed with `canon ask`. It is evaluated fresh instead
of being selected from the configured expectations.

## Cache

Stored pass history that lets `canon` avoid evaluating an expectation again
when a previous pass still applies to the current staged project state.

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

A time window during which a recent pass result can avoid being
re-proven for every small staged change. Cooldown is useful for broad review
expectations that are expensive to recheck on every commit.

## Cooldown result

A pass cached result derived from `last-pass.json` when the expectation has a
configured cooldown duration and the pass timestamp is still inside that window.

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

A 20-character base62 hash derived from the rendered expectation question, its
addressee, the expected answer, and a deterministic hash of the resolved
expectation instructions.

## Expectation report

The public `canon check` output for one expectation. Its full short ID identifies
the expectation, so writing that ID already reports something for it. Progress
markers and the result suffix add details when execution continues.

## Expected answer

The answer written in an expectation's `a` field. `canon` compares this value
to the observed answer using exact string equality.

## Expectation

A question, addressee, and expected answer that the project should satisfy. In
`.canon/check.yml`, a basic expectation has a `q` field and normally an `a`
field; shell expectations default `a` to `"0"`. An expectation may select
`to: agent`, `to: caller`, or `to: shell`, and may also provide an
`instructions` config value.

## Evaluator thread

An ephemeral evaluator interaction context whose history is not persisted across
`canon check` invocations. Within one check run, an evaluator thread may be
reused only when its effective startup configuration is compatible and its
rendered base/developer instructions match. Startup compatibility includes the
effective model, tools, plugins, and current visible workspace.

## Evaluator Prompt Boundary

The implementation-owned evaluator prompt and instruction templates are the
resource files under `resources/prompts/`. Config values such as expectation
`instructions` are human-authored canon data passed into those templates, not
additional implementation-owned prompt templates or instruction sources.
Rust code that supplies template data, renders it, or transports the result is
renderer plumbing, not additional evaluator prompt or instruction content.

## Invocation-local state

Information that `canon` retains and reads within one command invocation to
coordinate execution or make decisions, such as caches, work queues, and the
current report. Invocation-local state stays in memory. Temporary
filesystem-shaped inputs required by external interfaces—materialized evaluator
trees, prompt artifacts, and the isolated Codex runtime—are artifacts rather
than invocation-local state. Canon-owned roots have lifetime cleanup; a
caller-selected shared materialization cache follows its own retention contract.

## YAML expansion

`!include <relative-yaml-path>` inserts YAML read from the same config source,
relative to the including document. `!foreach` applies to a two-item sequence
of variable bindings and a YAML template. It renders one template copy for
every combination of binding values; binding strings containing `*` or `?`
select matching paths from the same source. `read(filename)` returns UTF-8 file
contents relative to the document containing the expansion.

## Observed answer

The answer produced by an expectation's addressee. Agent answers come from an
evaluator turn, caller answers come from one stdin line, and shell answers are
decimal exit-code strings. `canon` compares the observed answer to the expected
answer.

## Pre-commit hook

The Git hook installed by `canon pre-commit install`. It runs `canon gate` before a
commit and blocks staged changes that are not safe under the current canon
history.

## `canon check`

The command that evaluates expectations against the staged project state by
default inside a Git worktree. Outside a Git worktree it selects in-place mode
automatically. An explicit Git tree may be selected, and in-place mode evaluates
the current directory directly. It collects configured expectations, computes
cached pass results, and selects the uncached expectations for evaluation.
Explicit selectors force evaluation even when a cached result exists.

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
should not change. The `q-scope` config field is `auto` by default; an explicit
path list fixes that q-scope for every evaluator turn.

## Q-scope suggestion

An evaluator-provided scope claiming to be narrow enough to answer the current
question. It may or may not be a valid q-scope. For auto-scoped expectations,
`canon check` only attempts to verify a schema-valid suggestion when its induced
visible tree has at least 25% fewer files than the current visible tree.

## Scope narrowing

The runtime process for trying to store a narrower auto q-scope. When an
evaluator returns an answer with a q-scope suggestion, `canon check` may run an
independent interrogation under that suggested scope. If that verification
returns an answer, it becomes the final response under the verified q-scope;
otherwise the initial answer and scope remain final.

## Same-tree result

The expectation's last pass result when its stored `visibleTreeOid` matches the
current `visibleTreeOid` for that result's visible scope.

## Runtime log history

Every applicable runtime event is constructed through the command's diagnostic
writer. A positive `canon.logs.maxSize` and a persistent state namespace retain
JSON Lines copies as bounded cross-invocation history under `LOGS_DIR`
(`${CANON_STATE_DIR}/logs`).
Otherwise, the writer keeps the current invocation's events in memory. The
zero-size configuration also removes previously retained runtime log files when
a persistent state namespace is available. Persistent history is intentional
command output, not invocation-local working state.

## Selected expectations

The expectations that require evaluation work in the current `canon check` run.
Explicit selectors select matching expectations directly; the default
no-selector check subtracts cached expectations from collected expectations.

## Staged snapshot

The temporary Git-tracked project state that `canon check` evaluates by default
inside a Git worktree. It comes from the Git index, so unstaged and untracked
working tree files are not part of the snapshot. Explicit tree selection creates
an analogous snapshot; in-place checks do not use one.

## `visibleTreeOid`

The Git-compatible object ID of the tracked tree entries visible to the
evaluator after the visible scope is applied.

## Visible scope

The scope applied to a staged tracked tree for an evaluator interrogation. It is
formed from the interrogation q-scope plus configured ignore exclusions. A
fresh auto-scoped interrogation uses the `qScope` in `last-pass.json`, or full
project scope when no pass with a q-scope exists. An expectation configured with
a q-scope path list uses that list on every turn. Configured ignore patterns are
normalized as project-relative pathspec items, converted to excluding pathspec
items, and applied last.

## Visible tree

The scoped tree induced by a visible scope.

## Implementation Map

### In-place check boundary

An in-place check treats the current directory as filesystem contents, not as
a Git-backed checked tree. Repository-derived evaluation inputs—tree and object
IDs, refs, rendered diffs, tracking state, Git-root discovery, and file
hiding—are therefore absent from its evaluator context. Command-wide canon
configuration and canon-owned output paths are control-plane inputs; using them
for runtime-event retention does not make repository information part of the
checked subject or evaluator context.

`config_types::Expectation` and `check::core::ResolvedExpectation` carry
expectation questions, addressees, ranks, expected answers, the resolved
`instructions` config text, and target metadata from config loading into check
runs.

`scope` keeps scopes as Git pathspec lists: it normalizes repository paths,
forms visible scopes by appending configured ignore patterns as excluding
pathspec items, and matches tracked paths against those pathspec lists.

`git::visible_tree_oid` implements scoped tree and visible tree identity. It
collects the tracked entries induced by a scope, then computes the
repository-native Git tree object ID for those entries. When a scoped directory
already has a Git tree object, the implementation can reuse that object ID;
otherwise it serializes and hashes a synthetic tree object with the
repository's object hash algorithm. `materialization` uses that same OID when
materializing evaluator-visible trees.

`check::q_scope::initial_q_scope_for_check_run` forms the base q-scope from a
configured path list, or, for `auto`, from the last pass q-scope with full
project scope as fallback.
`xpec_state::last_result` reads and writes the status-specific last-result
files. `materialization::TreeMaterializer::materialize_visible_scope` then
applies the visible scope before creating the evaluator working tree.

`check::core::parse_evaluator_response_for_short_id` parses evaluator evidence
and any schema-appropriate `qScopeSuggestion` value.
`check::interrogation::policy` treats suggestions as unverified claims until an
independent verification turn accepts them.
`xpec_state` persists pass/fail last-result files, with `qScope`, `visibleScope`,
and tree OIDs for Git-backed results. The last pass may seed future auto-scoped
interrogations and supply a same-tree or cooldown cached result. Its bounded
global failure history supports same-HEAD recurring-failure feedback without
reading runtime logs.

`evaluator::protocol::prompt` renders the prompt templates stored under
`resources/prompts/` with MiniJinja. The developer-instructions template is
`resources/prompts/evaluator_developer_instructions.txt`; the renderer registers
the `json`, `shq`, `shargs`, and `sh` filters. It runs `sh` blocks from the
repository root outside in-place mode and from the checked directory in
in-place mode.

`check::interrogation::session::thread` owns evaluator-thread reuse. Its lookup
keys include the effective evaluator model and the runtime inputs that render
the evaluator instructions. They also split on live thread-start context
outside those rendered strings, including plugins, dynamic-tool availability,
and the current visible workspace.
