# Diff From

An expectation's `diff-from` value selects the left-hand tree used for prompt-rendered Git diffs.

The default value is `:checkpoint`.

`:checkpoint` resolves to the expectation's usable checkpoint, or to the check run's against tree when no usable checkpoint exists.

A checkpoint is usable only when the stored `checkedTreeOid` resolves to an existing Git tree in the repository object database.
If a stored checkpoint references a missing tree, the checkpoint is treated as corrupt stale state and the no-checkpoint behavior is used instead of failing with an error.

`:against-tree` resolves to the check run's against tree.

All other `diff-from` values are resolved using the same rules as `<TREE>` check options.
