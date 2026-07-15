# Progress Timeline

A **progress timeline** is the sequence of symbols recorded while an xpec is being evaluated.

A symbol is added after each full elapsed minute, and one final symbol is added for the final, possibly partial minute when the evaluation is ready to report.

The final minute may have an elapsed duration of 0 seconds.

Thus a completed timeline contains exactly `1 + floor(elapsed_seconds / 60)` symbols.

The symbol is chosen by the first matching rule for the minute:

```
×  a minute during which a turn attempt failed after exhausting its no-progress timeout
~  a minute during which the active turn attempt's no-progress timeout was accumulating
⇄  a minute during which an evaluator model fallback started
↻  a minute during which a short-ID mismatch triggered a fresh-thread retry
↗  a minute during which a full-scope retry started
⤡  a minute during which a q-scope verification started and that same verification returned `ScopeTooNarrow`
↖  a minute during which a q-scope verification returned `ScopeTooNarrow`
↘  a minute during which a q-scope verification started
.  a minute during which no higher-priority rule applied
```

`~` is chosen only while time is actively counting toward the turn attempt's no-progress timeout.

If that timeout is exhausted, the attempt fails and `×` is chosen.
