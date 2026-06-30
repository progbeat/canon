# Progress Timeline

A **progress timeline** is the sequence of progress markers emitted while evaluator work for an xpec is in progress.

Before the first progress marker is emitted, stdout is flushed immediately without printing a marker.

A marker is printed and flushed after each full elapsed minute, and one final marker is printed and flushed when the evaluator work is ready to report.

The final marker represents the elapsed interval since the previous minute marker and is printed even when that interval is 0 seconds.

Thus evaluator work prints exactly `1 + floor(elapsed_seconds / 60)` progress markers.

The marker is chosen by the first matching rule for the interval:

```
×  a minute during which a turn attempt failed after exhausting its no-progress timeout
~  a minute during which the active turn attempt's no-progress timeout was accumulating
⇄  a minute during which an evaluator model fallback started
↻  a minute during which a fresh-thread retry after a short-ID response error started
↗  a minute during which a full-scope retry started
↘  a minute during which a q-scope verification started
.  a minute during which no higher-priority marker applied
```

`~` is emitted only while time is actively counting toward the turn attempt's no-progress timeout.

If that timeout is exhausted, the attempt fails and `×` is emitted.
