# Turn Prompt Template

This is a prompt template for the turn prompt used for evaluator interrogations:

````jinja
{{ question }}
{%- if expectation.target == "diff" %}

This question targets the Git diff. If the Git diff doesn't prove otherwise, return the previous valid response:
```
{%- if last_pass %}
{
  "answer": {{ last_pass.response.answer|json }},
  "evidence": {{ last_pass.response.evidence|json }}
}
{%- else %}
{"answer": {{ expectation.a|json }}, "evidence": ""}
{%- endif %}
```
{%- endif %}
````

*When `last_pass` exists, the prompt intentionally targets the Git diff so the evaluator checks only whether the diff invalidates the previous pass.
For `target: diff` without `last_pass`, the expected-answer response is intentionally rendered as the previous valid response so evidence stays limited to the diff.*
