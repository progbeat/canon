# Dynamic Tools

**`canon.show`** is a dynamic tool exposed to evaluator interrogations and handled by the current `canon check` process.

During an interrogation, `canon.show` must not return the expectation being interrogated.

After applying that prohibition as if a `not:<current expectation ID>` selector were appended to the requested selectors, `canon.show` returns text with the same behavior and output format as `canon show` within the current check run.

If an evaluator thread receives `canon.show` output for an expectation, that thread must not later be reused for an interrogation of that expectation in the same check run.
