# Configuration

`canon` uses Git config to store configuration parameters.

| Parameter | Default | Meaning |
| --- | --- | --- |
| `canon.logs.maxSize` | `0M` | Size limit for `LOGS_DIR`. Values are byte counts and may use `M`, or `G` suffixes. |

When a parameter is not set, `canon` behaves exactly as if the default value from this table had been set explicitly.
