# Turn Prompt Template

This is a prompt template for the turn prompt used for evaluator interrogations:

````jinja
{{ question }}
{%- if git_diff and prev_answer %}

Before answering, check whether the Git diff invalidates the previous answer:
{{ prev_answer|json }}
If not, reuse it and stop.
{%- elif git_diff and expectation.target == "diff" %}

This question targets the Git diff. Inspect touched files and only the necessary context. If they do not prove otherwise, answer: {{ expectation.a|json }}.
{%- endif %}
````

`git_diff` is present when the checked tree differs from the against tree.

`prev_answer` contains the `answer` and `evidence` fields from a previous answer history record for the same expectation. Use the most recent eligible record at the against tree when one exists, otherwise use the most recent eligible record.

Only answer history records whose visible tree does not extend beyond the current visible tree are eligible for `prev_answer`.

The `answer` field corresponds to the history record's `observed` field.
