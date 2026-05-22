# canon

[![CI](https://github.com/progbeat/canon/actions/workflows/ci.yml/badge.svg)](https://github.com/progbeat/canon/actions/workflows/ci.yml)
[![Audit Status](https://github.com/progbeat/canon/actions/workflows/audit.yml/badge.svg)](https://github.com/progbeat/canon/actions/workflows/audit.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

When an AI agent misses the mark, there is always a human expectation it
violated. `canon` lets the human write those expectations down and make AI
agents iterate until all of them are satisfied.

That is how this project was built: no human-written implementation code,
just Codex working against `canon` until the repo satisfied its own canon.

See canon's own canon in [`.canon/check.yml`](.canon/check.yml).

## Install

Requires Git, Rust/Cargo, and the Codex CLI.

```sh
cargo install --git https://github.com/progbeat/canon
```

Cargo is the recommended install path on macOS, Linux, and Windows. Prebuilt
release binaries are not published yet.

To install the Codex skills, ask Codex:

```text
Install the Codex skills from https://github.com/progbeat/canon/tree/main/skills.
```

Restart Codex after installing the skills.

## Workflow

1. Ask Codex to implement a feature using `$canon-warden`.

2. If something is off, add the violated expectation to `.canon/check.yml`,
   then ask Codex to fix the project against the updated canon.

3. Iterate.

## How It Scales

Each expectation is checked in a sandboxed scope. When a question can be
answered from a smaller part of the repository, `canon` narrows and verifies
that scope. The scope is enforced with filesystem permissions, so the evaluator
cannot read project files outside the allowed scope.

That keeps larger canons practical: checks untouched by the staged change can
be skipped, while broad expectations can still use the full project when they
need it. `cooldown` gives broad review expectations, such as dead files, dirty
hacks, or idiomaticity, their own review cadence: after a recent pass, they do
not have to be re-proven for every small commit.

## Commands

```sh
canon init
```

Create `.canon/check.yml`.

```sh
canon check
```

Evaluate configured expectations against the staged project state.

```sh
canon check a7F K9m
```

Run selected expectations by unique ID prefix.

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

Bypass exact cached answer reuse, recheck fresh cooldown passes, continue after a
failed expectation, or use another config. By default, `canon check` stops after
the first final non-pass result.

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
