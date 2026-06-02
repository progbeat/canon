# Cached Result

The **same-tree record** for an expectation is the latest answer history record whose `visibleTreeOid` equals the current `visibleTreeOid`.

The **same-tree result** derived from that record is `pass` if its `observed` value matches the expectation’s current expected answer, and `fail` otherwise.

A **cooldown result** for an expectation exists when the expectation has a `cooldown`, its latest **answer history record** has a `pass` or `fail` result with a configured cooldown duration, and its `timestamp` is younger than that duration. The history record's result is `pass` if its `observed` value matches the expectation’s current expected answer, and `fail` otherwise. The cooldown result is `pass`.

A **cached result** for an expectation and Git state is the expectation's **same-tree result**, if one exists. Otherwise, it is the expectation's **cooldown result**, if one exists.

If neither exists, the expectation has no **cached result**.

## Cooldown Field

An expectation item may include an optional `cooldown` field:

```yaml
expectations:
  - q: "Are there any serious code quality issues that can be easily fixed?"
    a: "no"
    cooldown: 7d
```

`cooldown` is optional and intended only for project quality or other expensive expectations where frequent re-proving is not necessary.

`cooldown` may be a compact duration or a mapping with `pass` or `fail` durations. A compact duration is equivalent to `pass: <duration>`. Cooldown durations use compact positive duration syntax with exactly one integer and one unit. Supported units are `s`, `m`, `h`, `d`, and `w`, for seconds, minutes, hours, days, and weeks. Examples include `30m`, `4h`, `3d`, `2w`, and `cooldown: { fail: 21h, pass: 7d }`.
