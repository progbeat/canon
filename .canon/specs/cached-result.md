# Cached Result

A **same-tree result** for an expectation exists when the expectation's last pass result has a stored `visibleTreeOid` equal to the scoped tree OID of that result's `visibleScope` in the checked tree.
The same-tree result is that last pass result.

A **cooldown result** for an expectation exists when the expectation has a cooldown duration and the last pass result's `responseTimestamp` is younger than that duration.

The cooldown result is `pass`.

A **cached result** for an expectation and Git state is the expectation's **same-tree result**, if one exists. Otherwise, it is the expectation's **cooldown result**, if one exists.
Every cached result is `pass`.

If neither exists, the expectation has no **cached result**.

## Cooldown Field

An expectation item may include an optional `cooldown` field:

```yaml
xpecs:
  - q: "Can you find any serious code quality issues that can be easily fixed?"
    a: "no"
    cooldown: 7d
```

`cooldown` is optional and intended only for project quality or other expensive expectations where frequent re-proving is not necessary.

`cooldown` uses compact positive duration syntax with exactly one integer and one unit. Supported units are `s`, `m`, `h`, `d`, and `w`, for seconds, minutes, hours, days, and weeks. Examples include `30m`, `4h`, `3d`, and `2w`.
