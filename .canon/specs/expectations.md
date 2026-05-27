# Expectations

The `expectations` may contain explicit expectation items and generator items.

An explicit expectation item contains `q` and `a`:

```yaml
- q: "Does this behavior work?"
  a: "yes"
```

A generator item contains `include`:

```yaml
- include: "expectations/*.yml"
```

A generator item contains `path`, `q_template`, and `a`:

```yaml
- path: "specs/*.md"
  q_template: |
    {{content}}
    ---
    Is this specification implemented?
  a: "yes"
```

For every matched file, `q_template` is rendered by substituting `{{content}}` with the UTF-8 file contents to produce the generated expectation question.

Expectation items may include other fields not described here.
