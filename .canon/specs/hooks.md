# Check Hooks

A check config may contain an optional top-level `hooks` mapping.

A **check hook** is a check-config action triggered by a `canon check` lifecycle event.

`hooks` may contain `on-start` and `on-pass`.
Both hooks have the same format and behavior; only their trigger differs.

Each hook value is either a string or a mapping.
A string is shorthand for `print: <string>`.
Mapping fields are:

- `print`: text to print
- `confirm`: optional stdin line to require; never printed
- `repair-instruction`: optional instruction to print after the status when the hook blocks

If omitted, `repair-instruction` defaults to ``▷ Fix the blocker and run `canon check` again!``.

When a hook triggers, blank `print` skips the hook.
Otherwise `canon` prints `print`, then reads and exactly matches one stdin line if `confirm` is present.
A `confirm` mismatch adds one `blocked` outcome and is not an expectation result.

`on-start` runs after config/options validation, before the first interrogation.
`on-pass` runs after the last interrogation and before status output, only when `canon check` would otherwise pass.

Example:

```yaml
hooks:
  on-start: "Starting canon check.\n"
  on-pass:
    print: |
      Run the required linters. If they pass, type lint-pass:
    confirm: "lint-pass"
    repair-instruction: |
      Run the linters, fix any issues, then run `canon check` again.
```
