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
      "evidence": {
        "type": "string"
      },
      "answer": {
        "type": "string",
        "pattern": "^[-_a-z0-9]+$"
      },
      "error": {
        "type": "string",
        "enum": ["ScopeTooNarrow", "InvalidQuestion"]
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
      {"required": ["answer"]},
      {"required": ["error"]}
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

When an expectation's `q-scope` is a path list, every turn uses it, and each turn's schema omits `ScopeTooNarrow` from `error.enum`.

When an expectation's `q-scope` is a path list or an evaluation never hides files from evaluator turns, the schemas omit `qScopeSuggestion`.

When an expectation's `q-scope` is `auto` (default), its initial turn uses the `qScope` from the xpec's `last-pass.json`, or full project scope if no last pass result with `qScope` exists.

An invalid `qScopeSuggestion` returned by the evaluator agent is not used for narrowing.

The xpec's `thinking` setting configures evaluator thinking effort and is applied to each turn.
