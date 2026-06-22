# Human handoff

If an agent asked you to open this file, the notes below are project behavior
points the agent wanted to make visible without creating general repository
documentation.

## In-place checks

`canon check --in-place` still follows normal selected-expectation ordering, but
it does not use persistent state for cache reuse, stored q-scopes, cooldowns,
follow-up interrogations, or last-result writes.
