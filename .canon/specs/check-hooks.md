# Check Hooks

A `canon check` config may contain an optional top-level `hooks` mapping.

A **hook** is an action triggered by a `canon check` event.

`hooks` may contain the event keys `on-start` and `on-pass`.
Both events use the same hook format; only their trigger differs.

Each event value is either one hook or an ordered list of hooks.
Each hook is either a string or a mapping.
A string is shorthand for a hook with `print: <string>`.

Mapping fields are:

- `print`: optional text to print
- `input`: optional inline prompt text to print before reading one stdin line
- `exec`: optional command argv to run from the repository root, without shell expansion
- `cases`: optional mapping from input lines or exit codes to hook outcomes

A mapping hook must contain `print`, `input`, or `exec`.
`input` and `exec` are mutually exclusive.
`cases` is valid only when `input` or `exec` is present.
`input` and `exec` require `cases`.

The default repair instruction is ``▷ Fix the blocker and run `canon check` again!``.

When an event triggers, `canon` runs its hooks in order.
For each hook, `canon` prints `print`, if present, and appends one trailing newline.
Then `canon` prints `input`, if present, without appending a newline, reads one stdin line, and trims only the line ending before matching it.
If `exec` is present, `canon` runs the command and matches the process exit code.
If neither `input` nor `exec` is present, the hook continues after printing `print`.

`cases` keys are YAML scalar values normalized to their text form.
The key `_` is the fallback when no exact key matches.
Exact keys are tried before `_`.

`cases` values are hook outcomes:

- `!ok`: continue to the next hook
- `!block <repair-instruction>`: block immediately with the given repair instruction

A blocking hook adds one `blocked` outcome, skips later hooks for that event, and is not an expectation result.
If no `cases` entry applies, the hook blocks with the default repair instruction.

`on-start` runs after config/options validation, before the first interrogation.
`on-pass` runs after the last interrogation and before status output, only when `canon check` would otherwise pass.

Example:

```yaml
hooks:
  on-start:
    - input: "Confirm dependencies are installed [y/N] "
      cases:
        y: !ok
        _: !block "▷ Install dependencies, then run `canon check` again."
  on-pass:
    - exec: ["cargo", "test"]
      cases:
        0: !ok
        _: !block "▷ Run the tests, fix any issues, then run `canon check` again."
```
