# TODOs

- [ ] Switch to `ignore` crate instead of git pathspecs for scope matching.
- [ ] Detect if `canon` was invoked by an agent and forbid modifying files like README.md, AGENTS.md, etc.
- [ ] Instruct agent from `canon check` to do pre-/post- checks configured in `check.yml`.
- [ ] Collect stats such as number of consecutive passes, so that stable expectations can be identified and deselected in `canon show`.
- [ ] Allow maintainer agent to override scope for an expectation.
