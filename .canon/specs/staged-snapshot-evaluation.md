# Staged Snapshot Evaluation

`canon check` evaluates staged Git-tracked project content by default.

Evaluator sessions must not have access to project content outside the staged
Git-tracked snapshot, including unstaged changes and untracked files.

The real project working tree must remain unmodified by snapshot preparation,
evaluation, and cleanup.
