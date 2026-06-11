# Turn Prompt Template

This is a prompt template for the turn prompt used for evaluator interrogations:

````jinja
{{ question }}
{%- if expectation.target == "diff" %}

This question targets the Git diff. If the Git diff doesn't prove otherwise, answer `{{ expectation.a|json }}` with empty evidence.
{%- elif prev_answer %}

Before answering, check whether the Git diff invalidates the previous answer:
{{ prev_answer|json }}
Reuse the previous answer if it's still valid.
{%- endif %}
````

`prev_answer` contains the `answer` and `evidence` fields from a previous answer history record for the same expectation. Use the most recent eligible record at the against tree when one exists, otherwise use the most recent eligible record.

Only answer history records whose visible tree does not extend beyond the current visible tree are eligible for `prev_answer`.

The `answer` field corresponds to the history record's `observed` field.
