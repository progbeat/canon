# Log

`log` owns process-wide structured event logging, subscription, delivery, and general-purpose handlers; callers own event semantics and application integration.

## Events

An event has this extensible JSON-object structure:

```text
Level = "trace" | "debug" | "info" | "warn" | "error"  // increasing severity
Event = JsonObject {
    event: SingleLineString,
    timestamp?: NonNegativeInteger,  // original logging time, Unix milliseconds; supplied by log when absent
    level?: Level,
    ...: JsonValue,
}
```

An omitted `level` means `info` without adding a field to the object.

## Logging and subscription

```text
@ref("#log")
namespace log:
    emit(event: Event) -> void
    trace(name: SingleLineString, **fields: JsonValue) -> void
    debug(name: SingleLineString, **fields: JsonValue) -> void
    info(name: SingleLineString, **fields: JsonValue) -> void
    warn(name: SingleLineString, **fields: JsonValue) -> void
    error(name: SingleLineString, **fields: JsonValue) -> void
    on(pattern: String | List[String],
       callback: (Event) -> void,
       *, level: Level? = None) -> (() -> void)
    on(pattern: String | List[String], *, level: Level? = None) -> decorator
```

Level-named shortcuts copy `fields`, set `event=name` and `level` to the method name, and call `emit` once.

`emit`, `on`, decorator registration, and the returned unsubscribe callable begin with:

```python
debug_assert(current_thread() == main_thread(), 'Log operations require the main thread')
```

Ordinary logging omits `timestamp`; an explicit value preserves an original logging time, as in replay.
`emit(event)` preserves supplied fields, including `timestamp`; only an absent `timestamp` is filled with the current time.
Before invoking any callbacks, `emit` checks chronology in call order using process-wide `previous_timestamp`, initially `None`:

```python
debug_assert(previous_timestamp is None or event['timestamp'] >= previous_timestamp, 'Event timestamps must be non-decreasing')
previous_timestamp = event['timestamp']
```

`on(pattern, callback, ...)` adds a distinct registration of a function or callable handler until process exit or invocation of its returned unsubscribe callable, which removes only that registration.
Omitting the callback returns a decorator that registers and returns the decorated callable unchanged.
`level` is the minimum accepted severity; `None` accepts all levels.
`pattern` matches the entire event name, case-sensitively: `*` matches any sequence of characters, including empty; `?` matches exactly one character; all other characters are literal.
A list matches when any of its patterns matches; an empty list matches nothing.
Both filters must match.

`emit` synchronously invokes the callback once for each matching subscription active when the call begins.
Callback failures are reported to the caller after all matching callbacks have been attempted.
Callbacks receive the timestamped event read-only, including nested objects and arrays.
Callers keep callback resources alive while an `emit` call can still invoke them, including calls in flight.
There are no default subscriptions or recording.

Routing takes O(k) work in subscription count, where k is the number of matching subscriptions, excluding callback execution.
Resolving a new event-name/level pair or changed subscriptions is outside this bound.
