# Prompt Templates

**prompt template** is an evaluator prompt or instruction template rendered with MiniJinja.

Prompt template rendering starts with the repository root as the current working directory.

Prompt templates may execute shell commands during rendering with a `sh` block filter.

The rendered block body is the command executed during rendering.

The rendered terminal transcript starts with a command line prefixed by `$ `.
If `display=...` is provided, that command line uses exactly the `display` text.
It does not show the rendered block body.
If `display` is omitted, the command line shows the rendered block body.

When command output exceeds 8 KiB, the complete command stdout is saved to a temporary file owned by the evaluator thread.
The file must remain readable by that evaluator for the entire lifetime of the thread.
The rendered transcript shows only the output head, followed by exactly one truncation line in this format:

```
[truncated: showing first N of M lines; full output: <path>]
```

## Example

Template:

```jinja
{% filter sh(display="git diff --name-status --cached") %}
git diff --name-status {{ against_tree_oid|shq }} {{ checked_tree_oid|shq }}
{% endfilter %}
{% filter sh %}
echo "Hello, world!"
{% endfilter %}
```

Rendered:

```sh
$ git diff --name-status --cached
A	foo.txt
M	Cargo.toml
$ echo "Hello, world!"
Hello, world!
```

The rendered block body is executed during template rendering, but the transcript shows the `display` text.
