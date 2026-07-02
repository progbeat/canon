# Interrogation Policy

**interrogation** is a `canon check` evaluator turn for one expectation question.

Each evaluator task input is rendered from the turn prompt template.

An evaluator response must be a single JSON object accepted by the selected JSON Schema; the schema sent to the evaluator transport may differ from the selected schema only to fit transport support, must enforce each selected-schema restriction directly or through an equivalent supported form when possible, and leaves only the remaining restrictions to be enforced after parsing.

An interrogation is **restricted-scope** when its q-scope is not full project scope.
For this policy, **full project scope** means the q-scope `["."]` before configured ignore exclusions are applied to form the visible scope.

A restricted-scope interrogation uses this base response schema:

```json
{
  "type": "object",
  "propertyNames": {
    "pattern": "^[A-Za-z0-9]+$"
  },
  "additionalProperties": {
    "type": "object",
    "properties": {
      "error": {
        "type": "string",
        "enum": ["ScopeTooNarrow", "InvalidQuestion"]
      },
      "answer": {
        "type": "string",
        "pattern": "^[-_a-z0-9]+$"
      },
      "evidence": {
        "type": "string"
      },
      "qScopeSuggestion": {
        "type": "array",
        "minItems": 1,
        "items": {
          "type": "string",
          "minLength": 1,
          "pattern": "^[^\\r\\n]*$"
        }
      }
    },
    "required": ["evidence", "qScopeSuggestion"],
    "oneOf": [
      {"required": ["answer"], "not": { "required": ["error"] }},
      {"required": ["error"], "not": { "required": ["answer"] }}
    ],
    "additionalProperties": false
  }
}
```

An evaluator response must contain each unanswered interrogation requested by that evaluator turn as a property named by that interrogation's short ID; the property's value is the interrogation result.

A **short-ID response error** is an evaluator response that violates that requirement, or returns a short ID that was already answered on the same evaluator thread.
If a short-ID response error occurs after the evaluator thread has already produced a valid response, `canon check` discards that evaluator thread and retries the interrogation on a fresh evaluator thread.
If a short-ID response error occurs on the evaluator thread's first evaluator turn, `canon check` reports an error for that interrogation without a fresh-thread retry.

When an interrogation has full project scope, its response schema omits `ScopeTooNarrow` from `error.enum`.

When a check mode never hides files from evaluator interrogations, response schemas omit `qScopeSuggestion`, and `canon check` does not perform follow-up interrogations.

A fresh interrogation uses the `qScope` from the expectation's `last-pass.json`, or full project scope if no last pass result with `qScope` exists.

A **follow-up interrogation** is an additional interrogation required by this policy for the same expectation after the initial interrogation receives an evaluator response.

For one expectation, `canon check` performs at most one follow-up interrogation.

When a restricted-scope initial interrogation returns `error: "ScopeTooNarrow"`, the follow-up interrogation retries with full project scope, where `ScopeTooNarrow` is disabled.

When the final evaluator response has `error`, human review is required.

If the initial interrogation returns an answer, the follow-up interrogation verifies the suggested q-scope only when the visible tree induced by that suggestion contains at least 25% fewer files than the current visible tree.
The narrowed scope is accepted only when the verification result is pass -> pass, pass -> fail, or fail -> fail relative to the initial result.
It is rejected when verification would turn an initial fail into pass.

If the evaluator returns an invalid `qScopeSuggestion`, `canon check` does not attempt narrowing from it.

The expectation's `models` setting configures evaluator models in retry order.
`canon check` starts with the first model and tries later models in order only after technical evaluator failures.

The expectation's `thinking` setting configures evaluator thinking effort and is applied to each evaluator interrogation.
