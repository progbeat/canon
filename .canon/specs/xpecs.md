# Xpecs

A check config contains exactly one of `xpecs` or its alias `expectations`.

The value is a sequence of expectation items or nested sequences. Nested
sequences recursively contribute their expectation items in place.

An expectation item's `q` is always a string.
Its `to` is `agent`, `caller`, or `shell`, and defaults to `agent`:

```yaml
- q: "Does this behavior work?"
  a: "yes"

- to: caller
  q: "Have the local checks passed? [y/N]"
  a: "y"

- to: shell
  q: "python3 .canon/check.py"
```

`to` selects the addressee used to acquire the evaluation response.
Every configured field is resolved before evaluation regardless of `to`, even when it has no effect for the selected addressee.

`a` is required unless `to` is `shell`.
For `to: shell`, an absent or empty `a` resolves to `"0"`.
Resolved expected answers and answers in evaluation responses are strings, even when their source values are integers or other non-string scalar types.

For `to: shell`, Unix-like platforms use `/bin/sh -c`, and Windows uses `cmd.exe /D /S /C`.

Expectation items may include other fields not described here.
