# canon

[![CI](https://github.com/progbeat/canon/actions/workflows/ci.yml/badge.svg)](https://github.com/progbeat/canon/actions/workflows/ci.yml)
[![Audit Status](https://github.com/progbeat/canon/actions/workflows/audit.yml/badge.svg)](https://github.com/progbeat/canon/actions/workflows/audit.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

When an AI agent misses the mark, there is always a human expectation it
failed to meet. `canon` lets the human write those expectations down and make
AI agents iterate until all of them are satisfied.

That is how this project was built: no human-written implementation code,
just Codex working against `canon` until the repo satisfied its own canon.

## Install

Requires Git, Rust/Cargo, and the Codex CLI.

```sh
cargo install --git https://github.com/progbeat/canon
```

Cargo is the recommended install path on macOS, Linux, and Windows. Prebuilt
release binaries are not published yet.

To install the Codex skills, ask Codex:

```text
Install the Codex skills published with https://github.com/progbeat/canon.
```

Restart Codex after installing the skills.

## Workflow

1. Ask Codex to implement a feature using `$canon-warden`.

2. If something is off, add the unmet expectation to `.canon/check.yml`,
   then ask Codex to fix the project against the updated canon.

3. Iterate.

## How It Scales

Each expectation is checked in a sandboxed scope. When a question can be
answered from a smaller part of the repository, `canon` narrows and verifies
that scope. The scope is enforced with filesystem permissions, so the evaluator
cannot read project files outside the allowed scope.

That keeps larger canons practical: a same-tree result can be reused when the
expectation's `visibleTreeOid` still matches the evaluator-visible tree, while
`cooldown` gives broad review expectations, such as dead files, dirty hacks, or
idiomaticity, their own review cadence. A cached result is the newer of those
two reusable results.

When `canon check` is run with no selectors, cached failures are reported first
and must be fixed before fresh evaluation continues. If every cached result is a
pass, only expectations without a cached result are selected for evaluator work.

## Commands

```sh
canon init
```

Create `.canon/check.yml` from canon's embedded default template.

```sh
canon check
```

Evaluate configured expectations against the staged project state, using cached
results to avoid unnecessary evaluator work.

```sh
canon check a7F K9m
```

Explicitly evaluate selected expectations by unique ID prefix.

```sh
canon check -q "Can you find any practically exploitable security vulnerability?"
canon check -q "Does README.md sound clear?" -s README.md
```

Ask one uncached ad-hoc question. Add one or more `-s`/`--scope` paths to
debug the same question under a narrower evaluator scope.

```sh
canon check --ignore-cache
canon check --ignore-cooldown
canon check --all
canon check --config other-check.yml
canon check -c other-check.yml
```

Bypass same-tree cached results, bypass cooldown results, explicitly evaluate
all configured expectations, or use another config.

```sh
canon gate
```

Run the pre-commit gate manually.

```sh
canon hook install
```

Install the local pre-commit hook.

```sh
canon hook uninstall
```

Remove the local pre-commit hook.

## License

MIT
