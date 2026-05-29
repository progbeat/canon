# `canon check` Command

```sh
$ canon check --help
Check whether a Git tree meets project expectations written in the canon.

Usage: canon check [OPTIONS] [EXPECTATION]...

Arguments:
  [EXPECTATION]...  Expectation selectors: ID prefixes or full expectation IDs

Options:
  -c, --config <PATH>          Read expectations from this config file [default: .canon/check.yml]
  -q <QUESTION>                Ask one question
  -s, --scope <PATHSPEC>       Set the visible scope for the question
      --tree <TREE>            Check this Git tree [default: :staged]
      --against-tree <TREE>    Compare against this Git tree [default: HEAD]
      --keep-going             Continue after failures
      --ignore-cache           Re-evaluate expectations with cached results
      --ignore-cooldown        Re-evaluate expectations in cooldown
      --no-sandbox             Disable canon-managed sandboxing; caller is responsible for isolation
  -h, --help                   Print help

Examples:
  canon check
      Check staged content against all canon expectations.

  canon check a7F K9m
      Check canon expectations selected by ID prefix.

  canon check --ignore-cache a7F
      Freshly check one canon expectation.

  canon check --tree HEAD --against-tree HEAD~1 a7F
      Check one canon expectation on HEAD with comparison against the previous commit.

  canon check -q "Does the app expose Undo?"
      Ask a one-off question.

  canon check -q "Does the app expose Undo?" -s src/app.rs
      Ask a one-off question with a restricted visible scope.
```

*The `canon check --help` output may differ from this example in wording,
wrapping, spacing, and option order, while preserving the same command usage,
options, defaults, and common examples.*

## Expectation Result Output

When a check run processes expectations, stdout contains one result entry per
passing evaluated expectation, failed expectation, or errored expectation that
is emitted by the run.

For each passing evaluated expectation, stdout contains exactly one line:

```
P. OK
```

For each failed expectation, evaluated or cached, stdout contains exactly one block. The block has these required lines:

```
P. FAILED
<escaped question>
Expected: <escaped expected>
Observed: <escaped observed>
Evidence: <escaped evidence>
```

If a q-scope suggestion is available, the block appends this final line:

```
Suggested q-scope: <compact JSON array>
```

The line is omitted when no q-scope suggestion is available.

For each errored expectation, stdout contains exactly one block of lines:

```
P. ERROR
<escaped question>
Error: <escaped error>
Evidence: <escaped evidence>
```

`P` is the shortest prefix of the expectation's `ID` that uniquely identifies that expectation among the collected expectations.

Embedded control characters in the question, expected answer, observed answer,
error, and evidence are escaped before writing to stdout. Escaping prevents
evaluator-provided text from injecting additional stdout lines.

`Suggested q-scope` is rendered as a compact JSON array on one line.

## Token Usage Line

Then stderr contains exactly one token usage line:

```
Token usage: total=<n> input=<n> (+ <n> cached) output=<n> (reasoning <n>)
```

If token usage data is unavailable, every numeric field is `0`.

## Summary Line

Then stdout contains one summary line:

```
============================= <outcome-list> in <duration>s =============================
```

`outcome-list` is a comma-separated list of non-zero outcome counts in this
order: failed, error/errors, passed, skipped. If every count is zero, the
outcome list is `0 passed`. The outcome text is surrounded by spaces and padded
with `=` characters on both sides.

Outcome labels follow pytest pluralization: `failed`, `passed`, and `skipped`
are used for both singular and plural counts; `error` is used for one error and
`errors` for every other error count.

`passed` is the number of selected expectations whose result is `pass`.
`failed` is the number of expectations whose cached or evaluated result is `fail`.
`errors` is the number of selected expectations whose result requires human review.
`skipped` is the number of expectations not included in any category above (i.e. without output).

## Instructions to Agent

Let:

- `num_failed` be the number of expectations that failed in this run.
- `num_errors` be the number of expectations that encountered errors in this run.
- `num_regressions` be the number of expectations that changed from `pass` to any non-pass result compared to HEAD.
- `num_fixes` be the number of expectations that changed from any non-pass result (including missing) to `pass` compared to HEAD.

Assuming no Ctrl-C or other interruption, when `canon check` uses the default config, checks the `:staged` tree, and compares against `HEAD`, the check run may emit instructions for the agent that ran it like this:

```text
def print_agent_messages(num_failed, num_errors, num_fixes, num_regressions):
    num_issues = num_failed + num_errors
    if num_regressions > 0 or (num_issues > 0 and num_fixes == 0):
        print(f"▷ Fix the issues and run `canon check` again!")
        return
    if num_issues == 0 and num_fixes == 0:
        print("✓ All checks passed. Commit is allowed.")
        return
    assert num_fixes > 0
    passes_msg = f'1 pass' if num_fixes == 1 else f'{num_fixes} passes'
    print(f"▷ +{passes_msg} compared to HEAD. Commit the staged changes NOW!")
    if num_issues > 0:
        print(f"▷ Then fix the remaining issues and run `canon check` again!")
```
