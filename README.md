# canon

[![CI](https://github.com/progbeat/canon/actions/workflows/ci.yml/badge.svg)](https://github.com/progbeat/canon/actions/workflows/ci.yml)
[![Audit Status](https://github.com/progbeat/canon/actions/workflows/audit.yml/badge.svg)](https://github.com/progbeat/canon/actions/workflows/audit.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

When an AI agent misses the mark, there is always a human expectation it failed
to meet. `canon` lets the human write those expectations down and make AI agents
iterate until all of them are satisfied.

That is how this project was built: no human-written implementation code, just
Codex working against `canon` until the repository satisfied its own canon.

## Install

### Docker

Requires Git, Docker, and the Codex CLI.

Install the Docker [wrapper](https://github.com/progbeat/canon/tree/master/.canon/docker/scripts/canon)
as `canon` somewhere on `PATH`.

### Cargo

Requires Git, Rust/Cargo, and the Codex CLI.

```sh
cargo install --git https://github.com/progbeat/canon
```

Cargo is the recommended install path on macOS, Linux, and Windows. Prebuilt
release binaries are not published yet.

## Codex Skills

To install the Codex skills, ask Codex:

```text
Install the Codex skills from `https://github.com/progbeat/canon/tree/master/skills`.
```

Restart Codex after installing the skills.

## Workflow

1. Ask Codex to implement a feature using `$canon-warden`.

2. If something is off, add the unmet expectation to `.canon/check.yml`, then
   ask Codex to fix the project against the updated canon.

3. Iterate.

## Commands

```sh
canon init
```

Create `.canon/check.yml` from `canon`'s embedded default template.

```sh
canon hook install
```

Install the local pre-commit hook.

```sh
canon hook uninstall
```

Remove the local pre-commit hook.

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
```

Ask one uncached ad-hoc question. Add one or more `-s`/`--scope` paths to debug
the same question under a narrower evaluator scope.

```sh
canon gate
```

Run the pre-commit gate manually.

## License

MIT
