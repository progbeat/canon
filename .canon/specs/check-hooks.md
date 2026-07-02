# Check Hooks

A `canon check` config may contain an optional top-level `hooks` mapping.

A **hook** is an action triggered by a `canon check` event.

`hooks` may contain the event keys `on-start` and `on-pass`.
Both events use the same hook format; only their trigger differs.

Each event value is either one hook or an ordered list of hooks.
Each hook is either a string or a mapping.
A string is shorthand for a hook with `print: <string>`.
Mapping fields are:

- `print`: text to print
- `confirm`: optional stdin line to require; never printed
- `repair-instruction`: optional instruction to print after the status when the hook blocks

If omitted, `repair-instruction` defaults to ``▷ Fix the blocker and run `canon check` again!``.

When an event triggers, `canon` runs its hooks in order.
For each hook, blank `print` skips that hook.
Otherwise `canon` prints `print`, then reads and exactly matches one stdin line if `confirm` is present.
The printed text is not newline-normalized; if `print` does not end with a newline, confirm input appears on the same terminal line.
A `confirm` mismatch blocks immediately, adds one `blocked` outcome, skips later hooks for that event, and is not an expectation result.

`on-start` runs after config/options validation, before the first interrogation.
`on-pass` runs after the last interrogation and before status output, only when `canon check` would otherwise pass.

Example:

```yaml
hooks:
  on-start:
    - "Starting canon check.\n"
    - print: "Confirm dependencies are installed [y/N] "
      confirm: "y"
  on-pass:
    - print: "Run the required linters. If they pass, type lint-pass: "
      confirm: "lint-pass"
      repair-instruction: |
        ▷ Run the linters, fix any issues, then run `canon check` again.
```
