# `canon check` Order

## Default Policy

`canon check` evaluates selected expectations in ascending order by `rank`.
Expectations with the same `rank` are evaluated in descending order by the timestamp of their latest fail result.
Use rank `0` when unset and the `canon check` start time when an expectation has no fail results.
