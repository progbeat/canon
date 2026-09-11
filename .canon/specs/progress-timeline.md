# Progress Timeline

A **progress timeline** logs ordered heartbeat events after each full elapsed minute and once for the final interval when evaluation is ready to report, even if that interval is zero seconds.
It continues while the evaluator waits for input or other operations.

For each interval, the timeline logs:

```python
log.info('xpec.evaluation.heartbeat', id=xpec.id, activity=activity, marker=marker)
```

The **progress activity** and its marker come from the first matching rule for the interval:

| Activity | Marker | Condition |
| --- | --- | --- |
| `timeout` | `×` | A turn attempt failed after exhausting its no-progress timeout |
| `no_progress` | `~` | The no-progress timeout accumulated continuously for the entire full minute |
| `model_fallback` | `⇄` | An evaluator model fallback started |
| `short_id_mismatch_retry` | `↻` | A short-ID mismatch triggered a fresh-thread retry |
| `full_scope_retry` | `↗` | A full-scope retry started |
| `scope_verification_started_and_rejected` | `⤡` | A q-scope verification started and that same verification returned `ScopeTooNarrow` |
| `scope_too_narrow` | `↖` | A q-scope verification returned `ScopeTooNarrow` |
| `scope_verification` | `↘` | A q-scope verification started |
| `running` | `.` | No higher-priority rule applied |

`stop()` completes the final update and all logging callbacks before returning; no later updates occur.
