# Cached Result

If the last fail record has `visibleTreeOid` equal to the scoped tree OID of its `visibleScope` in the checked tree, the **same-tree result** for that expectation is `fail`.

Otherwise, if the last pass record has `visibleTreeOid` equal to the scoped tree OID of its `visibleScope` in the checked tree, the **same-tree result** for that expectation is `pass`.

A **cooldown result** for an expectation exists when at least one of the following is true:

- the expectation has a `pass` cooldown duration, and the last pass record's `responseTimestamp` is younger than that duration;
- the expectation has a `fail` cooldown duration, and the last fail record's `responseTimestamp` is younger than that duration.

The cooldown result is `pass`.

A **cached result** for an expectation and Git state is the expectation's **same-tree result**, if one exists. Otherwise, it is the expectation's **cooldown result**, if one exists.

If neither exists, the expectation has no **cached result**.

## Cooldown Field

An expectation item may include an optional `cooldown` field:

```yaml
expectations:
  - q: "Can you find any serious code quality issues that can be easily fixed?"
    a: "no"
    cooldown: 7d
```

`cooldown` is optional and intended only for project quality or other expensive expectations where frequent re-proving is not necessary.

`cooldown` may be a compact duration or a mapping with `pass` or `fail` durations. A compact duration is equivalent to `pass: <duration>`. Cooldown durations use compact positive duration syntax with exactly one integer and one unit. Supported units are `s`, `m`, `h`, `d`, and `w`, for seconds, minutes, hours, days, and weeks. Examples include `30m`, `4h`, `3d`, `2w`, and `cooldown: { fail: 21h, pass: 7d }`.
