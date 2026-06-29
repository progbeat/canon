# Clarify In-Place "Selected Expectations"

The in-place canon says "Selected expectations must be valid without Git-tree,
diff, cache, or path-hiding behavior." Evaluators interpreted this both as
CLI-selected expectations and as all configured expectations in consecutive
`canon check` runs.

Please clarify whether "selected" means the CLI-resolved selected set for the
current run. The implementation currently follows that interpretation.
