# `!foreach` YAML Tag

The `!foreach` tag applies to a two-item sequence:

```yaml
!foreach
- path: "specs/**.py"
- source: "{{ path }}"
  contents: "{{ read(path) }}"
```

The first item maps `path` to a glob.

The second item, called the **foreach template**, may be any YAML node.

The glob is resolved relative to the directory containing the YAML document and against the same source from which the document was read.

For each matched path, `path` is bound to the matched path and every string scalar in a copy of the foreach template is rendered. During rendering, `read(path)` returns the bound path's UTF-8 contents from the same source.

`!foreach` resolves to a sequence containing the rendered copies.
