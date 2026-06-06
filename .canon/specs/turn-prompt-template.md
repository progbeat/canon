# Turn Prompt Template

This is a prompt template for the turn prompt used for evaluator interrogations:

````jinja
{{ question }}{% if against_tree_answer %}

Your previous answer at HEAD:
{{ against_tree_answer|json }}
{% endif %}
````

`against_tree_answer` contains the `answer` and `evidence` fields from the most recent answer history record for the same expectation on the against tree with the same visible scope as the current interrogation. The `answer` field corresponds to the history record's `observed` field.
