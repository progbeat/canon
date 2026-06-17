# Diff From

An expectation's `diff-from` value selects the left-hand tree used for prompt-rendered Git diffs.

The default value is `:checkpoint`.

`:checkpoint` resolves to the expectation's checkpoint, or to the check run's against tree when no checkpoint exists.

`:against-tree` resolves to the check run's against tree.

All other `diff-from` values are resolved using the same rules as `<TREE>` check options.
