# `canon gate` Command

`canon gate` is the fast pre-commit check for staged changes.

`canon gate` decides pass/fail using the following logic:

```
def gate(num_regressions):
    if num_regressions > 0:
        return Fail
    if any staged path is under .canon/**:
        if every staged path is under .canon/**:
            return Pass
        else:
            return Fail
    return Pass
```

Every `canon gate` failure prints an actionable message.
