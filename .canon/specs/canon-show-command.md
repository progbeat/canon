# `canon show` Command

```sh
$ canon show --help
Show canon expectations.

Usage: canon show [OPTIONS] [SELECTOR]... [-- <PATHSPEC>...]

Arguments:
  [SELECTOR]...  Expectation selectors: ID prefixes or full expectation IDs
  [PATHSPEC]...  Limit output to expectations affected by changes matching these pathspecs

Options:
      --tree <TREE>  Use this Git tree for pathspec filtering [default: :staged]
  -h, --help         Print help
```

*The `canon show --help` output may differ from this example in wording, wrapping, spacing, and option order.*

Without selectors, `canon show` starts from all collected expectations.

With selectors, `canon show` starts from expectations matched by the same selector matching rules as `canon check`.

Pathspecs after `--` further narrow expectations to those whose visible tree OID for the selected Git tree would change if every tracked file matched by those pathspecs changed.

## Output

Each displayed expectation is written to stdout as:

```text
<short ID>.
<escaped question>
Expected: <escaped expected answer>
```

Questions and expected answers are escaped the same way as `canon check` stdout.

Displayed expectations use the same ordering policy as `canon check`.
