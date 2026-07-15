# Interrogation Policy

Each turn's task input is rendered from the turn prompt template.

Each turn must return a single JSON object accepted by the selected JSON Schema; the schema sent to the evaluator transport may differ from the selected schema only to fit transport support, must enforce each selected-schema restriction directly or through an equivalent supported form when possible, and leaves only the remaining restrictions to be enforced after parsing.

A turn is **restricted-scope** when its q-scope is not full project scope.
For this policy, **full project scope** means the q-scope `["."]` before configured ignore exclusions are applied to form the visible scope.

A restricted-scope turn uses this response schema:

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
    "required": ["qScopeSuggestion"],
    "oneOf": [
      {"required": ["answer", "evidence"], "not": { "required": ["error"] }},
      {"required": ["error"], "not": { "anyOf": [{"required": ["answer"]}, {"required": ["evidence"]}] }}
    ],
    "additionalProperties": false
  }
}
```

The object returned by a turn must contain each unanswered interrogation requested by that turn as a property named by that interrogation's short ID; the property's value is that interrogation's evaluation response.

A **short-ID mismatch** occurs when a turn violates that requirement or returns a short ID that was already answered on the same evaluator thread.
If a short-ID mismatch occurs after the evaluator thread has already produced a valid turn, that evaluator thread is discarded and the interrogation is retried on a fresh evaluator thread.
If a short-ID mismatch occurs on the evaluator thread's first turn, the interrogation produces an evaluation response with `error` without a fresh-thread retry.

When a turn has full project scope, its schema omits `ScopeTooNarrow` from `error.enum`.

When an evaluation never hides files from evaluator turns, the schemas omit `qScopeSuggestion`, and the interrogation does not perform follow-up turns.

An interrogation's initial turn uses the `qScope` from the xpec's `last-pass.json`, or full project scope if no last pass result with `qScope` exists.

A **follow-up turn** is an additional turn required by this policy after the initial turn produces that interrogation's evaluation response.

An interrogation has at most one follow-up turn.

When a restricted-scope initial turn returns `error: "ScopeTooNarrow"`, the follow-up turn retries with full project scope, where `ScopeTooNarrow` is disabled.

When the final evaluation response has `error`, human review is required.

When the initial turn produces a passing answer, the follow-up turn verifies the suggested q-scope only when the visible tree induced by that suggestion contains at least 25% fewer files than the current visible tree.
The narrowed scope is accepted only when that verification returns an answer.

An invalid `qScopeSuggestion` returned by the evaluator agent is not used for narrowing.

The xpec's `models` setting configures evaluator models in fallback order.
A later model may be tried only after a technical evaluator failure and any applicable retries of the current model.

The xpec's `thinking` setting configures evaluator thinking effort and is applied to each turn.
