# Configuration

`canon` uses Git config to store configuration parameters.

| Parameter | Default | Meaning |
| --- | --- | --- |
| `canon.logs.maxSize` | `0M` | Size limit for `LOGS_DIR`. Values are byte counts and may use `M`, or `G` suffixes. |
| `canon.threadReuse.carryoverTokenTarget` | `10000,30000` | Target range `MIN,MAX` for carried-over tokens. |

When a parameter is not set, `canon` uses the default value.
