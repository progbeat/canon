# Turn Prompt Template

This is a prompt template for the turn prompt used for evaluator interrogations:

````jinja
{{ {xpec.short_id: xpec.q}|json }}
{%- if xpec.target == "diff" %}
# This question targets the Git diff. Evaluate whether the diff changes the answer. Use this prior evaluation if it still holds: `{% if xpec.diff_from == ":checkpoint" and last_pass -%}
{{ {
  "answer": last_pass.response.answer,
  "evidence": last_pass.response.evidence,
  "qScopeSuggestion": ["."]
}|json }}
{%- else -%}
{{ {
  "answer": xpec.a,
  "evidence": "",
  "qScopeSuggestion": ["."]
}|json }}
{%- endif %}`
{%- endif %}
````

*Prompt hints may be intentionally false when that makes the evaluator's answer search more effective.
For `target: diff`, the default evaluation is presented as a prior evaluation so the evaluator focuses on whether the diff changes the answer.*
