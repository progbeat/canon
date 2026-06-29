# Show pending expectations

## Request

Add a `canon show` or related inspection mode that lists collected xpecs that
do not have a corresponding current `last-pass.json`.

## Why

After canon updates, agents need to align implementation before running
`canon check`. A pending-only view would make it easier to find newly added or
cache-missing xpecs without starting evaluator work.
