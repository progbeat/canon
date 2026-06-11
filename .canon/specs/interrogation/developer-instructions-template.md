# Developer Instructions Template

This is a prompt template for the developer instructions:

```jinja
{% if expectation.instructions|trim -%}
{{ expectation.instructions|trim }}

{% endif -%}
{% filter sh(display="git diff --numstat --cached") %}
git diff --numstat {{ against_tree_oid|shq }} {{ checked_tree_oid|shq }}
{% endfilter %}

{% filter sh(display=("git diff --cached -- " ~ (visible_scope|shargs))) %}
git diff {{ against_tree_oid|shq }} {{ checked_tree_oid|shq }} -- {{ visible_scope|shargs }}
{% endfilter %}

$ enter-sandbox
You are now in the read-only sandbox. Git commands are unavailable.
{{ num_invisible_files }} project files are hidden because they are likely unnecessary to answer the question.
```

`visible_scope` is the visible scope of the first interrogation on that thread.

`num_invisible_files` is the number of files in the checked tree minus the number of files in the visible tree.
