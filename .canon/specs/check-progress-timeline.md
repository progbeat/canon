# Check Progress Timeline

For each expectation result emitted to stdout, `canon check` prints a **progress timeline** between the short ID and the final result status.

The timeline starts with `.` printed immediately.
This first marker is not an elapsed-time marker.

For evaluated expectations, one more marker is printed and flushed every minute until the result is ready.
The marker is chosen by the first matching rule:

```
×  a minute during which a turn attempt failed after idle, transport, or app-server failure
~  a minute during which no app-server activity was observed
↗  a minute during which a full-scope retry started
↘  a minute during which a q-scope verification started
.  a minute during which app-server activity was observed
```

**App-server activity** is any app-server protocol message received for the active turn attempt. `~` shows idle time accumulating toward that attempt's no-progress timeout.

`↗` and `↘` are used only for the minute in which the corresponding turn starts.
`×` is turn-attempt-level; retry or fallback work may continue after it.
