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

### AGENTS.md

Copy the
[`Canon` section from this repository's `AGENTS.md`](https://github.com/progbeat/canon/blob/master/AGENTS.md?plain=1#L12-L56)
into your project's `AGENTS.md`.

### Docker

Requires Git, Docker, curl, and Codex (Desktop or CLI).

This is the recommended install path for most users. `canon` itself was built
by AI agents and has not been reviewed line by line by a human. The Docker
[wrapper](https://github.com/progbeat/canon/tree/master/.canon/docker/scripts/canon) is the small human-reviewed trust boundary: it runs the published
Docker image in an unprivileged container, drops Linux capabilities, disables
new privileges, uses a read-only container filesystem, keeps Codex credentials
read-only on the host, and only gives the container the repository access needed
for `canon` commands.

The image sets `CANON_NO_SANDBOX=true` so every evaluator command uses the
container as its external isolation boundary. `canon check` additionally exposes
the public `--no-sandbox` option, which the entrypoint injects while forwarding
the user-supplied arguments unchanged.

Before installing, review the Docker
[wrapper](https://github.com/progbeat/canon/tree/master/.canon/docker/scripts/canon)
on GitHub. Install it only if you are comfortable trusting that wrapper as the
host-side boundary.

After reviewing the wrapper, install it as the `canon` executable in a directory
on your `PATH`. For example, to install it in `~/.local/bin`:

```sh
mkdir -p "$HOME/.local/bin"
curl -fsSL https://raw.githubusercontent.com/progbeat/canon/master/.canon/docker/scripts/canon -o "$HOME/.local/bin/canon"
chmod +x "$HOME/.local/bin/canon"
```

If `~/.local/bin` is not already on `PATH`, add it to your shell configuration:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

After that, `command -v canon` should print a path under `$HOME/.local/bin`.

### Cargo

Requires Git, Rust/Cargo, and the Codex CLI. Host-native Linux installations
also require [Bubblewrap](https://github.com/containers/bubblewrap) on `PATH`
with user namespaces available.

```sh
cargo install --git https://github.com/progbeat/canon
```

Use Cargo when developing `canon` or when you intentionally want a host-native
binary. Prebuilt release binaries are not published yet.

## Agent Skills

To install the skills from this repository, ask Codex:

```text
Install the agent skills from `https://github.com/progbeat/canon/tree/master/skills`.
```

## Workflow

1. Ask Codex to implement a feature.

2. If something is off, add the unmet expectation to `.canon/check.yml`, then
   ask Codex to fix the project against the updated canon.

3. Iterate.

Canon changes are the human's responsibility. Before committing them, it is
recommended to use `$canon-guidelines` for review.

## Commands

```sh
canon init
```

Create `.canon/check.yml` from `canon`'s embedded default template.

```sh
canon pre-commit install
```

Install the local pre-commit hook.

```sh
canon pre-commit uninstall
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

Explicitly evaluate selected expectations by unique ID prefix or full ID. These
selectors are expectation IDs, not 1-based expectation numbers.

```sh
canon ask "Can you find any practically exploitable security vulnerability?"
```

Ask one uncached ad-hoc question.

```sh
canon gate
```

Run the pre-commit gate manually.

## License

MIT
