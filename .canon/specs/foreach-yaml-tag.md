# `!foreach` YAML Tag

The `!foreach` tag applies to a two-item sequence:

```yaml
!foreach
- path: ["glossary.md", "specs/**.py"]
  mode: ["brief", "thorough"]
- source: "{{ path }}"
  mode: "{{ mode }}"
  contents: "{{ read(path) }}"
```

The first item maps one or more variable names to a value or a sequence of values.
A single value is equivalent to a one-item sequence containing that value.

The second item, called the **foreach template**, may be any YAML node.

Strings containing `*` or `?` are globs resolved relative to the directory containing the YAML document and matched against the same source from which the document was read.
All other values are literals.

For every combination of the resulting values, every string scalar in a copy of the foreach template is rendered with the corresponding variable bindings.
`read(value)` returns the named file's UTF-8 contents from the same source, resolving literal filenames relative to the directory containing the YAML document.
A combination selected more than once contributes one rendered copy.

`!foreach` resolves to a sequence containing the rendered copies.
