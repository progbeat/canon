# Cooldown

An expectation item may include an optional `cooldown` field:

```yaml
expectations:
  - q: "Are there any dirty hacks that can be avoided?"
    a: "yes"
    cooldown: 7d
```

`cooldown` is optional and intended only for project quality or other expensive
expectations where frequent re-proving is not necessary.

`cooldown` values use compact positive duration syntax with exactly one integer
and one unit. Supported units are `s`, `m`, `h`, `d`, and `w`, for seconds,
minutes, hours, days, and weeks. Examples include `30m`, `4h`, `3d`, and `2w`.

Cooldown applies only when the latest valid history record for that expectation
has `result` equal to `pass` and its `timestamp` is younger than the configured
cooldown duration.

When cooldown applies, the expectation is removed from the selected expectation
set.
