# Diff From

An expectation's `diff-from` value selects the left-hand tree used for prompt-rendered Git diffs.

The default value is `:checkpoint`.

`:checkpoint` resolves using the first matching rule:

1. If the expectation has no checkpoint whose stored `checkedTreeOid` resolves to an existing Git tree, use the check run's against tree.
2. If the expectation has a path-list `q-scope` and the files changed within it relative to the checkpoint are not a subset of those changed relative to HEAD, use HEAD.
3. Otherwise, use that checkpoint tree.

`:against-tree` resolves to the check run's against tree.

All other `diff-from` values are resolved using the same rules as `<TREE>` check options.
