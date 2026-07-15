# `canon check --in-place`

`canon check --in-place` is used to check the current directory directly.

When `canon check` is run outside a Git worktree, in-place mode is selected automatically.

In-place mode checks the current directory as filesystem contents and ignores Git information if the directory has any.
In-place mode does not compute tree OIDs, so in-place last result files omit tree OID fields.
Evaluator context is not based on rendered Git diffs.
Files in the checked directory are not hidden.

The evaluator agent is started in the checked directory.

`--tree`, `--against-tree` are rejected in in-place mode.

Selected expectations must be valid without Git-tree, diff, cache, or path-hiding behavior.
Configured `diff-from`, `target`, `cooldown`, and `ignore` are invalid in in-place mode.
