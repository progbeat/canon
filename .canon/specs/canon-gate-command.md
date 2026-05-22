# `canon gate` Command

`canon gate` is the fast pre-commit check for staged changes.

`canon gate` decides pass/fail using the following logic:

```
def gate(selected_expectations):
    if any staged path is under .canon/**:
        if every staged path is under .canon/**:
            return Pass
        else:
            return Fail
    for each expectation in selected_expectations:
        prev_res = cached result for expectation at HEAD
        curr_res = cached result for expectation in the staged Git tree
        if prev_res is Pass and curr_res is not Pass:  # if regression:
            return Fail
    return Pass
```

Every `canon gate` failure prints an actionable message.
