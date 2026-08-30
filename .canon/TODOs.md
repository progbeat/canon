# TODOs

- [ ] Switch to the `ignore` crate instead of git pathspecs for scope matching.
- [ ] Collect stats such as the number of consecutive passes so that stable expectations can be identified and deselected in `canon show`.
- [ ] Allow the caller to override the scope for an expectation.
- [ ] `canon show --failed --pending`.
- [ ] `canon show --jsonl`.
- [ ] Update `check-q-scope` to search xpecs by text.
- [ ] Add SemVer and a CHANGELOG.
- [ ] Instruct the evaluator agent to treat marked tests and assertions as the source of truth.
- [ ] Add an xpec that checks that tests are directly derived from xpecs.
- [ ] When the result is FAIL, retry with a smarter model to reduce false negatives.
- [ ] Ask whether there is a user-facing lie.
- [ ] Assign a distinct, stable **error identifier** at each source location that creates an evaluation error. Preserve the identifier while the error is propagated so that it uniquely identifies its creation site.
- [ ] Add a setting to specify a branch that is always supposed to pass `canon check`. It can be used for optimizations.
