# `canon check` Order

## Default Policy

`canon check` evaluates selected expectations in ascending order by `rank`, using rank `0` when unset.
Expectations with the same `rank` are evaluated in descending order by the timestamp of their latest fail result.
If an expectation has no fail results, use the Unix epoch when it has a pass result, and the `canon check` start time when it has no results.
