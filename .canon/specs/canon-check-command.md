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
      Check all expectations whose IDs start with a7F or K9m.

  canon check not:a7F not:K9m
      Check all expectations except those whose IDs start with a7F or K9m.

  canon check --tree HEAD --against-tree HEAD~1 a7F
      Check expectations whose IDs start with a7F on HEAD against the previous commit.
```

*The `canon check --help` output may differ from this example in wording,
wrapping, spacing, and option order, while preserving the same command usage,
options, defaults, and common examples.*

The behavior of `canon check` follows this shape:

```python
evaluate = import(ref="#evaluate")
emit_check_feedback = import(ref="#emit_check_feedback")

def echo_off(fn):
    global interactive_posix_terminal
    interactive_posix_terminal = (platform == POSIX and stdin.is_terminal() and stdout.is_terminal())
    if not interactive_posix_terminal:
        return fn
    def wrapper(*args, **kwargs):
        ... # disable ECHO & ECHONL
        try:
            return fn(*args, **kwargs)
        finally:
            ... # restore
    return wrapper

@echo_off
def canon_check():
    ... # do everything needed to prepare for evaluation
    try:
        for xpec in check_order_policy(selected_expectations):
            evaluation = evaluate(xpec)
            ... # perform any other required work
            if evaluation["status"] == FAIL and not keep_going:
                break
    finally:
        emit_token_usage()
        emit_summary()
        if (
            not in_place
            and config has its command-default value
            and tree has its command-default value
            and against_tree has its command-default value
        ):
            emit_check_feedback(...)
        ...
```

## Token Usage Line

The token usage line is written to stderr as:

```
token-usage: ref-cost={:.2}$ total={} input={} ({} cached) output={} (reasoning {})
```

If token usage data is unavailable, every numeric field is `0`.

## Summary Line

The summary line is written to stdout as:

```
============================= <outcome-list> in <duration>s =============================
```

`outcome-list` is a comma-separated list of non-zero outcome counts in this order: failed, passed, pending. If every count is zero, the outcome list is `0 passed`.
The outcome text is surrounded by spaces and padded with `=` characters on both sides.

Outcome labels `failed`, `passed`, and `pending` are used for both singular and plural counts.

`passed` is the number of expectations whose status is `PASS`.
`failed` is the number of expectations whose status is `FAIL`.
`pending` is the number of expectations for which the check run has no result.

Each collected expectation is counted exactly once in passed, failed, or pending.

## Runtime Logs

`canon check` logs runtime events including:

- check lifecycle;
- expectation outcomes;
- evaluator communication;
- model and fallback failures;
- review-required diagnostics;
- token usage when available.

Evaluator communication events include tasks and returned messages before interpretation or repair, with context linking each exchange to the check run.

Evaluator communication events identify the boundary between the command and the evaluator agent.

Evaluator thread events include creation, reuse, and the effective evaluator instructions used for each thread.

When usage data is available for an evaluator turn, token usage events include input tokens, cached input tokens, output tokens, and reasoning output tokens with enough context to match the usage to that turn.
