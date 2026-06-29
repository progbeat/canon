# Prompt Templates

**prompt template** is an evaluator prompt or instruction template rendered with MiniJinja.

Outside in-place mode, prompt template rendering starts with the repository root as the current working directory.
In in-place mode, prompt template rendering starts with the checked directory as the current working directory.

Prompt templates may execute shell commands during rendering with a `sh` block filter.

The rendered block body, after trimming leading and trailing whitespace, is the command executed during rendering.

The rendered terminal transcript starts with a command line prefixed by `$ `.
If `display=...` is provided, that command line uses exactly the `display` text.
It does not show the rendered block body.
If `display` is omitted, the command line shows the trimmed rendered block body.

When command output exceeds 8 KiB, the complete command stdout is saved to a temporary file. Within a single `canon check` invocation, the path is deterministic from the complete stdout content, so identical output is saved once and referenced by the same path.
The file must be readable by the evaluator that receives the path for the lifetime of the thread.
The rendered transcript shows only the output head, followed by exactly one truncation line in this format:

```
[truncated: showing first N of M lines; full output: <path>]
```

## Example

Template:

```jinja
{% filter sh(display="git diff --name-status") %}

   echo "A	foo.txt\nM	Cargo.toml"


     {% endfilter %}
{% filter sh %} echo "Hello, world!" {% endfilter %}
```

Rendered:

```sh
$ git diff --name-status
A	foo.txt
M	Cargo.toml
$ echo "Hello, world!"
Hello, world!
```

The rendered block body is executed during template rendering, but the transcript shows the `display` text.
