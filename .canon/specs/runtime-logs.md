# Runtime Logs

`LOGS_DIR` is `${CANON_STATE_DIR}/logs`.

If logging is enabled, `canon` commands may record runtime log events under `LOGS_DIR`.

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

Event names and event-specific schemas are implementation-defined as long as the
required information remains available from the logs.

Runtime logs should avoid recording derived information when the corresponding raw structured information is already recorded.
