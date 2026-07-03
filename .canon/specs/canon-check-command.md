# `canon check` Command

```sh
$ canon check --help
Check whether project files meet human expectations written in the canon.

Usage: canon check [OPTIONS] [SELECTOR]...

Arguments:
  [SELECTOR]...  Expectation selectors: <ID-PREFIX> or not:<ID-PREFIX>

Options:
  -c, --config <PATH>          Read expectations from this config file [default: .canon/check.yml]
      --tree <TREE>            Check this Git tree [default: :staged]
      --against-tree <TREE>    Compare against this Git tree [default: HEAD]
      --in-place               Check the current directory directly
      --keep-going             Continue after failures
      --no-sandbox             Disable canon-managed sandboxing; caller is responsible for isolation
  -h, --help                   Print help

Examples:
  canon check
      Check staged content against all canon expectations.

  canon check a7F K9m
      Check canon expectations selected by ID prefix.

  canon check not:a7F not:K9m
      Check all expectations except those whose IDs start with a7F or K9m.

  canon check --tree HEAD --against-tree HEAD~1 a7F
      Check one canon expectation on HEAD with comparison against the previous commit.
```

*The `canon check --help` output may differ from this example in wording,
wrapping, spacing, and option order, while preserving the same command usage,
options, defaults, and common examples.*

## Expectation Result Output

When a check run reports expectation results, each reported passing evaluated
expectation, failed expectation, or errored expectation is emitted as one stdout
result entry.

For each passing evaluated expectation, the result entry is exactly one line:

```
<short ID><progress timeline> OK
```

For each failed expectation, evaluated or cached, the result entry is exactly one block. The block has these required lines:

```
<short ID><progress timeline> FAILED
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

For each errored expectation, the result entry is exactly one block of lines:

```
<short ID><progress timeline> ERROR
<escaped question>
Error: <escaped error>
Evidence: <escaped evidence>
```

Embedded control characters in the question, expected answer, observed answer,
error, and evidence are escaped before writing to stdout. Escaping prevents
evaluator-provided text from injecting additional stdout lines.

`Suggested q-scope` is rendered as a compact JSON array on one line.

## Token Usage Line

The check run emits exactly one token usage line to stderr:

```
Token usage: total=<n> input=<n> (+ <n> cached) output=<n> (reasoning <n>)
```

If token usage data is unavailable, every numeric field is `0`.

## Summary Line

The check run emits one stdout summary line:

```
============================= <outcome-list> in <duration>s =============================
```

`outcome-list` is a comma-separated list of non-zero outcome counts in this order: blocked, failed, error/errors, passed, pending. If every count is zero, the outcome list is `0 passed`.
The outcome text is surrounded by spaces and padded with `=` characters on both sides.

Outcome labels follow pytest pluralization: `failed`, `blocked`, `passed`, and `pending` are used for both singular and plural counts; `error` is used for one error and `errors` for every other error count.

`blocked` is the number of hooks that blocked completion.
`passed` is the number of expectations whose result is pass.
`failed` is the number of expectations whose result is fail.
`errors` is the number of expectations that encountered errors during evaluation in this run.
`pending` is the number of expectations that do not yet have an evaluated or cached result.

Each collected expectation is counted exactly once in passed, failed, errors, or pending.

## Instructions to Agent

Assuming no Ctrl-C or other interruption, when `canon check` runs without expectation selectors, with the default config, on the `:staged` tree, and against `HEAD`, it may emit instructions for the agent that ran it like this:

```python
def print_agent_messages(blocked, failed, errors, num_new_passes, num_regressions, num_pending):
    """
    :param blocked: Blocked hooks.
    :param failed: Short IDs of failed expectations.
    :param errors: Short IDs of expectations that encountered errors in this run.
    :param num_new_passes: Number of xpecs classified as **new pass**.
    :param num_regressions: Number of xpecs classified as **regression**.
    :param num_pending: Number of pending expectations.
    """
    if blocked:
        assert len(blocked) == 1
        print(blocked[0].repair_instruction)
        return
    issues = failed + errors
    if num_regressions > 0 or (len(issues) > 0 and num_new_passes == 0):
        _repair_instructions(issues)
        print(f"▷ Fix the issues and run `canon check` again!")
        return
    if len(issues) == 0 and num_new_passes == 0:
        assert num_pending == 0
        print("✓ All checks passed. Commit is allowed.")
        return
    assert num_new_passes > 0
    passes_msg = f'1 pass' if num_new_passes == 1 else f'{num_new_passes} passes'
    print(f"▷ +{passes_msg}. Commit the staged changes NOW!")
    if len(issues) > 0:
        _repair_instructions(issues)
        print(f"▷ Then fix the remaining issues and run `canon check` again!")
    else:
        assert num_pending == 0

def _repair_instructions(issues):
    assert len(issues) > 0
    # These issues were already shown in `canon check` output, so don't show them again to save tokens.
    selectors = ' '.join(f'not:{x}' for x in issues)
    print("❕ Verify that the evidence supports the observed answer and answers the expectation question; treat unsupported evidence as a readability issue.")
    print(f"❕ Plan the repair, then run `canon show {selectors} [not:<ALREADY_IN_CONTEXT_EXPECTATION>]... -- <PATHSPEC>...` for the planned edit paths to identify expectations that may be affected.")
    print("❕ Use the matching expectations to avoid regressions while fixing the issues.")
```

## Runtime Logs

`canon check` logs runtime events including:

- check lifecycle;
- expectation outcomes;
- evaluator communication;
- model and fallback failures;
- review-required diagnostics;
- token usage when available.

Evaluator communication events include tasks and responses before interpretation or repair, with context linking each exchange to the check run.

Evaluator communication events identify the boundary between the command and the evaluator.

Evaluator thread events include creation, reuse, and the effective evaluator instructions used for each thread.

When usage data is available for an evaluator turn, token usage events include input tokens, cached input tokens, output tokens, and reasoning output tokens with enough context to match the usage to that turn.
