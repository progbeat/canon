# Turn Prompt Template

This is a prompt template for the turn prompt used for evaluator interrogations:

````jinja
{{ question }}
{%- if expectation.target == "diff" %}

This question targets the Git diff. Evaluate whether the diff changes the answer. Return the previous valid response if it still holds:
```
{%- if last_pass %}
{
  "answer": {{ last_pass.response.answer|json }},
  "evidence": {{ last_pass.response.evidence|json }},
  "qScopeSuggestion": ["."]
}
{%- else %}
{"answer": {{ expectation.a|json }}, "evidence": "", "qScopeSuggestion": ["."]}
{%- endif %}
```
{%- endif %}
````

*Prompt hints may be intentionally false when that makes the evaluator's answer search more effective.
For `target: diff`, the default response is presented as a previous valid response so the evaluator focuses on whether the diff changes the answer.*
