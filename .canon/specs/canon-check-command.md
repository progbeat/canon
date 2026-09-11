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
      --max-failures <COUNT>   Stop after COUNT failures; 0 means unlimited [default: 1]
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

`--max-failures` accepts a non-negative integer.

The command's behavior and status-log subscriptions follow this pseudocode:

```python
from contextlib import ExitStack

log = import(ref="#log")
setup_logging = import(ref="#setup_logging")
evaluate = import(ref="#evaluate")
emit_check_feedback = import(ref="#emit_check_feedback")
print_timeline_marker = import(ref="#print_timeline_marker")
print_token_usage = import(ref="#print_token_usage")

status_events = ['check.start', 'xpec.evaluation.*', 'check.finish']

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
    ... # resolve arguments
    setup_logging()
    with ExitStack() as lifetime:
        if CANON_STATE_DIR is not None:
            lifetime.enter_context(exclusive_lock(CANON_STATE_DIR / 'check.lock'))

        ... # collect, select, and prepare for evaluation
        if git_backed:
            run_path = CANON_STATE_DIR / 'runs' / '<timestamp>-<unique>.jsonl'
            status_file = lifetime.enter_context(log.FileHandler(run_path))
            log.on(status_events, status_file)

        log.on('xpec.evaluation.start', _on_evaluation_start)
        log.on('xpec.evaluation.heartbeat', print_timeline_marker)
        log.on('xpec.evaluation.finish', _on_evaluation_finish)
        log.on('check.finish', _on_check_finish)

        failures = []
        try:
            log.info('check.start', ...)
            if git_backed:
                atomically_replace_symlink(
                    CANON_STATE_DIR / 'runs' / 'latest.jsonl',
                    target=run_path.name,  # relative target; check.start is already flushed
                )

            for xpec in check_order_policy(selected_expectations):
                evaluation = evaluate(xpec)
                ... # perform any other required work
                if evaluation['status'] == FAIL:
                    failures.append({'id': xpec.id, 'shortId': xpec.short_id})
                    if len(failures) == max_failures:
                        break
        finally:
            log.info('check.finish', ...)

def _on_check_finish(event):
    print_token_usage(event)
    emit_summary(event)
    if (
        not in_place
        and config has its command-default value
        and tree has its command-default value
        and against_tree has its command-default value
    ):
        emit_check_feedback(event)

def _on_evaluation_start(event):
    xpec = get_xpec_by_id(event['id'])
    print(xpec.short_id, end='', flush=True)

def _on_evaluation_finish(event):
    xpec = get_xpec_by_id(event['id'])
    if event['status'] == 'pass':
        print(' OK')
        return
    print(' FAIL')

    error = event.get('error')
    if xpec.to == CALLER and error is None:
        print('expected:', xpec.a)
        return

    if xpec.to == SHELL and error is None:
        for line in event['evidence'].splitlines():
            print('│', line)
        print(f"Command exited with code {event['observed']} (expected {xpec.a}).")
        return

    print(escape_inline(xpec.q))
    if error is None and (diff_from_oid := xpec.diff_from_oid) is not None:
        short_diff_from_oid = ...  # Git abbreviation of diff_from_oid
        print('diff-from:', short_diff_from_oid, f"({xpec.diff_from})")
    if error is not None:
        print('error:', error)
    if error is None and xpec.a:
        print('expected:', xpec.a)
    if error is None:
        print('observed:', event.get('observed'))
    if (evidence := event.get('evidence')) is not None:
        print('evidence:', escape_inline(evidence))
```

## Summary Line

`result` is the run's final `pass` or `fail`.
`summary` partitions collected expectations into non-negative integer counts `passed`, `failed`, and `pending` (no result), plus non-negative elapsed `durationSeconds`.
`emit_summary(event)` writes:

```
============================= <outcome-list> in <duration>s =============================
```

`outcome-list` is a comma-separated list of non-zero outcome counts in this order: failed, passed, pending. If every count is zero, the outcome list is `0 passed`.
`duration` is `event['summary']['durationSeconds']`; the labels stay plural even for one.
Surround the outcome text with spaces and pad both sides with `=`.

## Runtime activity

Owning components log structured runtime activity as it occurs, independently of file recording:

- check lifecycle;
- evaluation events;
- evaluator communication;
- model and fallback failures;
- review-required diagnostics;
- token usage when available.

Evaluator communication events identify the command/agent boundary and carry tasks and returned messages before interpretation or repair, linked to the check run.

Evaluator thread events include creation, reuse, and the effective evaluator instructions used for each thread.

Per-turn token usage events include available input, cached input, output, and reasoning output counts, linked to that turn.

## Status Log

Only `canon check` acquires the exclusive lock; `exclusive_lock` waits for its current holder, and the operating system releases it if the holder exits.

A **status log** is the append-only per-run JSONL file written by a Git-backed `canon check`. Recording and check serialization require a resolvable `CANON_STATE_DIR`.
Bounded retention removes old status logs while preserving the active file and the target of `runs/latest.jsonl`.
