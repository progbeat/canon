# TODOs

- [ ] Switch to `ignore` crate instead of git pathspecs for scope matching.
- [ ] Detect if `canon` was invoked by an agent and forbid modifying files like README.md, AGENTS.md, etc.
- [ ] Collect stats such as number of consecutive passes, so that stable expectations can be identified and deselected in `canon show`.
- [ ] Allow the caller to override scope for an expectation.
- [ ] `canon show --failed --pending`.
- [ ] `canon show --jsonl`.
- [ ] Update `check-q-scope` to search xpecs by text.
- [ ] SemVer & CHANGELOG.
- [ ] preset: A+B+C
- [ ] Instruct evaluator agent to treat tests and asserts as source of truth.
- [ ] Add an xpec that checks that tests are directly derived from xpecs.
- [ ] When the result is FAIL, retry with a smarter model to reduce false negatives.
- [ ] Forbid feedback in `error` being not constructive, e.g. "unparsable".
- [ ] Ask if there's user facing lie.
