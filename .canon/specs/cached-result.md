# Cached Result

If `$XPECS_DIR/$ID/last-fail.json` has `visibleTreeOid` equal to the scoped tree OID of its `visibleScope` in the checked tree, the **same-tree result** for that expectation is `fail`.

Otherwise, if `$XPECS_DIR/$ID/last-pass.json` has `visibleTreeOid` equal to the scoped tree OID of its `visibleScope` in the checked tree, the **same-tree result** for that expectation is `pass`.

A **cooldown result** for an expectation exists when the expectation has a `cooldown`, the most recently updated of `$XPECS_DIR/$ID/last-pass.json` and `$XPECS_DIR/$ID/last-fail.json` has a status with a configured cooldown duration, and its `responseTimestamp` is younger than that duration. The cooldown result is `pass`.

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
