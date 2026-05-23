# `canon check` Command

Evaluator agent sessions do not have read access to `LOGS_DIR`.

`canon check` writes human-readable output to stdout.

This section specifies the stdout and stderr format for check runs that process
expectations. It is not an exhaustive output contract for modes that do not
process expectations.

For each passing expectation that is written to stdout, stdout contains exactly
one line:

```
P. OK
```

For each failed expectation, stdout contains exactly six lines:

```
P. FAILED
<escaped question>
Expected: <escaped expected>
Observed: <escaped observed>
Evidence: <escaped evidence>
Scope: <compact JSON array>
```

For each errored expectation, stdout contains exactly five lines:

```
P. ERROR
<escaped question>
Expected: <escaped expected>
Observed: <escaped observed>
Evidence: <escaped evidence>
```

`P` is the shortest prefix of the expectation's 20-character `ID` that uniquely
identifies that expectation among all the expectations.

Embedded control characters in question, expected answer, observed answer, and
evidence are escaped before writing to stdout. Newline is rendered as `\n`.
Escaping prevents evaluator-provided text from injecting additional stdout
lines.

`Scope` is rendered as a compact JSON array on one line.

Skipped expectations emit no per-expectation stdout. Failing results are never
skipped.

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

`passed` is the number of non-skipped selected expectations whose final result
is `pass`.
`failed` is the number of selected expectations whose final result is `fail`
because the evaluator returned an answer that was parsed successfully and did
not match the expected answer.
`errors` is the number of selected expectations whose final result requires
human review.
`skipped` is the number of non-selected expectations.

## Instructions to Agent

Assuming no Ctrl-C or other interruption, when `canon check` is called without
arguments, the check run may emit instructions for the agent that ran it like
this:

```
def print_agent_messages(num_failed, num_errors, num_fixes, num_regressions):
    """
    num_failed: The number of expectations that failed in this run.
    num_errors: The number of expectations that encountered errors in this run.
    num_fixes: The number of expectations that changed from non-OK to OK compared to HEAD.
    num_regressions: The number of expectations that changed from OK to non-OK compared to HEAD.
    """
    num_non_ok = num_failed + num_errors
    if num_regressions > 0 or num_non_ok > 0 and num_fixes == 0:
        print(f"▷ Fix the issues and run `canon check` again!")
        return
    if num_non_ok == 0 and num_fixes == 0:
        print("✓ All checks passed. Commit is allowed.")
        return
    assert num_fixes > 0
    passes_msg = f'1 pass' if num_fixes == 1 else f'{num_fixes} passes'
    print(f"▷ +{passes_msg} compared to HEAD. Commit the staged changes NOW!")
    if num_non_ok > 0:
        print(f"▷ Then fix the remaining issues and run `canon check` again!")
```
