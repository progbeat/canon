# Prompt Templates

**prompt template** is an evaluator prompt or instruction template rendered with MiniJinja.

Prompt template rendering starts with the repository root as the current working directory.

Prompt templates may execute shell commands during render with a `sh` block filter.
The `sh(display=...)` argument sets the command text shown in the rendered terminal transcript; the block body is the command Canon executes.
If `display` is omitted, the executed command text is also shown.

When command output exceeds 32 KiB, it is saved to a temporary file, and the rendered transcript shows only the output head, followed by exactly one truncation line in this format:

```
[truncated: showing first N of M lines; full output: <path>]
```

## Example

Template:
```jinja
{% filter sh(display="git diff --name-status --cached") %}
git diff --name-status {{ against_tree_oid|shq }} {{ checked_tree_oid|shq }}
{% endfilter %}
```

Rendered:
```sh
$ git diff --name-status --cached
A	foo.txt
M	Cargo.toml
```
