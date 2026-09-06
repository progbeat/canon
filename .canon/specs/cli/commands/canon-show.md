# `canon show` Command

```sh
$ canon show --help
Show canon expectations.

Usage: canon show [OPTIONS] [SELECTOR]... [-- <PATHSPEC>...]

Arguments:
  [SELECTOR]...  Expectation selectors: <ID-PREFIX> or not:<ID-PREFIX>
  [PATHSPEC]...  Limit output to expectations affected by changes matching these pathspecs

Options:
      --tree <TREE>  Use this Git tree for pathspec filtering [default: :staged]
  -h, --help         Print help
```

*The `canon show --help` output may differ from this example in wording, wrapping, spacing, and option order.*

Without selectors, `canon show` starts from all collected expectations.

With selectors, `canon show` starts from expectations matched by the same selector matching rules as `canon check`.

Pathspecs after `--` further narrow expectations to those that have a visible tree OID for the selected Git tree and whose OID would change if every tracked file matched by those pathspecs changed.

## Output

Each displayed expectation is written to stdout as:

```text
<short ID>
q: <escaped q>
a: <a>
```

The `q` and `a` are escaped the same way as `canon check` stdout.

Displayed expectations use the same ordering policy as `canon check`.
