# Interrogation Policy

**interrogation** is a `canon check` evaluator turn for one expectation question.

Each evaluator task input is exactly the question string.

An evaluator response must be a single JSON object matching this JSON Schema:

```json
{
  "type": "object",
  "properties": {
    "answer": {
      "type": "string",
      "minLength": 1,
      "pattern": "^[^\\r\\n]*$"
    },
    "error": {
      "type": "string",
      "enum": ["insufficient-evidence", "invalid-question", "unparsable"]
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
```

An unparsable evaluator response is invalid JSON or does not match the evaluator response schema. The contents of schema-valid fields do not make a response unparsable. `canon check` treats an unparsable evaluator response as `{"error":"unparsable","evidence":"<parse-error>"}`.

A fresh interrogation uses the stored q-scope for that expectation, or full project scope if no q-scope is stored.

When an interrogation that does not use full project scope returns `error: "insufficient-evidence"`, `canon check` retries with full project scope. The restricted `insufficient-evidence` is not final.

When the final evaluator response has `error`, human review is required.

If the evaluator returns an answer, `canon check` verifies the suggested q-scope with an independent interrogation only when the visible tree induced by that suggestion contains at least 25% fewer files than the current visible tree.
The narrowed scope is accepted and stored only when the verification interrogation produces a valid response with an `answer` field.

If the evaluator returns an invalid `qScopeSuggestion`, `canon check` does not attempt narrowing from it.

The expectation's `models` setting configures evaluator models in retry order.
`canon check` starts with the first model and tries later models in order only after technical evaluator failures.

The expectation's `thinking` setting configures evaluator thinking effort and is applied to each evaluator interrogation.
`thinking` does not affect evaluator thread reuse.
