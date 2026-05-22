# Runtime Logs

`LOGS_DIR` is `${CANON_STATE_DIR}/logs`.

`canon check` records runtime log events under `LOGS_DIR`.

Runtime logs are JSON Lines files. Each non-empty line is one complete JSON object.

The active runtime log file is `LOGS_DIR/0.jsonl`. Older runtime logs are
retained as rotated files in the same directory.

Each log record is appended and flushed as soon as the record is produced.

Each runtime log event contains these fields:

```text
timestamp
level
event
```

`timestamp` is UTC and records when the event is produced.

`level` is a single-line severity label.

`event` is a single-line event name.

Additional fields depend on the event type and remain extensible.
Event-specific data is recorded as structured JSON fields, not encoded into a
human-readable message string.

Runtime logs expose enough structured information to understand the check
lifecycle, expectation outcomes, evaluator communication, model and fallback
failures, review-required diagnostics, and token usage when available.

Evaluator communication logs record the boundary between `canon check` and the
evaluator. Runtime logs contain enough structured information to audit evaluator
tasks and responses before interpretation or repair, with enough context to
understand how each exchange fits into the check run.

Runtime logs should also make evaluator thread creation and reuse understandable
when that behavior matters for debugging. The effective evaluator instructions
used for a thread should be inspectable from the logs.

For each evaluator turn, runtime logs should contain enough data to determine
input tokens, cached input tokens, output tokens, and reasoning output tokens.
The usage can be matched to that evaluator turn. If the turn checks an expectation,
the usage can also be matched to that expectation.

Event names and event-specific schemas are implementation-defined as long as the
required information remains available from the logs.

Runtime logs should avoid recording derived information when the corresponding raw structured information is already recorded.
