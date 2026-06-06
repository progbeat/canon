# Developer Instructions Template

This is a prompt template for the developer instructions:

````jinja
{{ static_developer_instructions }}

```
{% filter sh(display="git diff --numstat --cached") %}
git diff --numstat {{ against_tree_oid|shq }} {{ checked_tree_oid|shq }}
{% endfilter %}

$ sandbox --read-only --scope {{ visible_scope|json }}
Sandbox is enabled. Hidden files: {{num_invisible_files}}.
```
````

`visible_scope` is the visible scope of the first interrogation on that thread.

`num_invisible_files` is the number of files in the checked tree minus the number of files in the visible tree.
