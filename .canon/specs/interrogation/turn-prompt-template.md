# Turn Prompt Template

This is a prompt template for the turn prompt used for evaluator interrogations:

````jinja
{{ {xpec.short_id: xpec.q}|json }}
{%- if xpec.target == "diff" %}
Evaluate only the visible files affected by the diff; use other visible files as context!
{%- endif %}
````

*Prompt hints may be intentionally false when that makes the evaluator's answer search more effective.
For `target: diff`, the default evaluation is presented as a prior evaluation so the evaluator focuses on whether the diff changes the answer.*
