# Interrogation Policy

An interrogation is a `canon check` evaluator turn for one expectation under one
enforced scope.

The enforced scope is supplied in evaluator thread developer instructions.

Each evaluator task input is exactly the expectation question string.

Expected answers are not included in evaluator task inputs.

The evaluator response format is exactly one JSON object with these keys in this
order and no extra keys:

```text
answer
evidence
scope
```

`answer` is a single-line string.

`evidence` is a string citing supporting files or code. Evidence citations are
separate from scope.

`scope` is either `["."]` or a JSON array of normalized repository-relative path
strings.

Evaluator Codex threads are ephemeral to one `canon check` invocation.

Withing one invocation, the same Codex thread can be reused for interrogations
with the same enforced scope to improve context retention and reduce token usage.

A fresh interrogation starts from the latest accepted scope for that
expectation, or `["."]` if there is no accepted scope yet.

When an interrogation returns `idk`, `canon check` retries with `["."]` scope and
does not treat the restricted `idk` as final when full-scope evidence can answer.

When an interrogation with a full scope returns `idk`, human review is required.

When the evaluator response is `malformed`, unparseable, or has empty evidence,
human review is required.

If the evaluator returns a correct or incorrect answer and a strictly narrower scope,
`canon check` verifies that strict-subset scope with an independent interrogation
on that narrower scope. The narrowed scope is accepted only when the observed answer
is unchanged or incorrect.

`canon check` uses `agent.model.primary` as the primary evaluator model.
Configured `agent.model.fallbacks` are tried in order only after technical
app-server or model failures such as `usageLimitExceeded`.

`agent.thinking` configures the default evaluator thinking effort. An explicit
expectation item may include a `thinking` value to override `agent.thinking` for
that expectation. Generated expectations inherit the generator item's `thinking`
when present, otherwise they use `agent.thinking`. The effective thinking effort
is applied to the evaluator interrogation, but it does not require a separate
evaluator thread when the enforced scope is the same.
