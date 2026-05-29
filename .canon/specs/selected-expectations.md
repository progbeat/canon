# Selected Expectations

**Collected** expectations are all expectations derived from config.

**Cached** expectations are the subset of collected expectations that have a
**cached result**.

**Selected** expectations are expectations that require evaluation. The selected set is mutable while command arguments, cached results, and other inputs are applied, but should not be modified after evaluation starts.

Default selected expectation logic is:

```
if expectation selectors are provided:
    selected = select expectations matching the selectors  # explicit user request
elif every cached result is a pass:
    # Cached results are insufficient to determine the final result, so evaluation is needed.
    selected = collected - cached
else:
    selected = empty  # cached failures must be fixed first
```
