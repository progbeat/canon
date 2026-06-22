# `canon check --in-place`

`canon check --in-place` is used to check the current directory directly.

When `canon check` is run outside a Git worktree, in-place mode is selected automatically.

In-place mode is evaluated against the directory as it exists at runtime.
No separate Git-backed checked tree or visible tree is created.
Evaluator context is not based on rendered Git diffs.
Files in the checked directory are not hidden by q-scope, stored q-scope, expectation `ignore`, scope narrowing, or retry behavior.

The evaluator is started in the checked directory.

Persistent check state is not used in in-place mode.
Cached results, stored q-scopes, checkpoints, cooldowns, and xpec last-result state are ignored.
Runtime diagnostic logs may be written, but existing diagnostic logs must not affect check behavior.

`--tree`, `--against-tree`, and `-s`/`--scope` are rejected in in-place mode.

Selected expectations must be valid without Git-tree, diff, cache, or path-hiding behavior.
Configured `diff-from`, `target`, `cooldown`, and `ignore` are invalid in in-place mode.
Ordinary q/a expectations and evaluator settings such as `instructions`, `models`, `thinking`, and `plugins` remain valid.

Generator and include items remain valid in in-place mode.
Their `path` and `include` patterns are treated only as config-expansion inputs.
