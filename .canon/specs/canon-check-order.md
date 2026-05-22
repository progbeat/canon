# `canon check` Order

## Default Policy

`canon check` runs the selected expectations by descending time of each
expectation's latest final non-pass result.

A final non-pass result includes both failed results and human-review/error
results.

Expectations with no final non-pass history keep the existing stable order.

By default, `canon check` stops after the first final non-pass result.

`canon check --all` checks the full already-selected expectation set.
