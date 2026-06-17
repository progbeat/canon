# Check Progress Timeline

For each expectation result emitted to stdout, `canon check` prints a **progress timeline** between the short ID and the final result status.

The timeline starts with `.` printed immediately.
This first marker is not an elapsed-time marker.

For evaluated expectations, one more marker is printed and flushed every minute until the result is ready.
The marker is chosen by the first matching rule:

```
×  a minute during which a turn attempt failed after exhausting its no-progress timeout
~  a minute during which the active turn attempt's no-progress timeout was accumulating
↗  a minute during which a full-scope retry started
↘  a minute during which a q-scope verification started
.  a minute during which no higher-priority marker applied
```

`~` is emitted only while time is actively counting toward the turn attempt's no-progress timeout. If that timeout is exhausted, the attempt fails and `×` is emitted.
