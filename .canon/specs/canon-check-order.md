# `canon check` Order

## Default Policy

`canon check` evaluates selected expectations in descending order by the timestamp of their latest non-pass result.

A non-pass result includes both failed results and human-review/error results.

By default, once `canon check` starts evaluating selected expectations, it stops after the first evaluated non-pass result.

If an expectation has no non-pass results, use the Unix epoch timestamp.
