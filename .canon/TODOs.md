# TODOs

- [ ] Switch to `ignore` crate instead of git pathspecs for scope matching.
- [ ] Detect if `canon` was invoked by an agent and forbid modifying files like README.md, AGENTS.md, etc.
- [ ] Collect stats such as number of consecutive passes, so that stable expectations can be identified and deselected in `canon show`.
- [ ] Allow the caller to override scope for an expectation.
- [ ] `canon show --failed --pending`.
- [ ] `canon show --jsonl`.
- [ ] Update `check-q-scope` to search xpecs by text.
