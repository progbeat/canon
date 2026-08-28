# Glossary

**xpec** (**expectation**) is the basic unit of the canon: a formalized human expectation consisting of a question `q`, its addressee `to`, and its expected answer `a`.

**ID** is a 20-character base62 hash derived from the tuple of the rendered `q`, `to`, `a`, and a deterministic hash of the expectation instructions.

**short ID** is the shortest prefix of an expectation's **ID** that uniquely identifies that expectation among the collected expectations.

**evidence** is evaluator-provided text that directly justifies an answer or, when no answer is provided, explains why the evaluator cannot provide one.

**expectation instructions** are the resolved `instructions` value for an expectation, or empty text when none is configured.

**checkpoint** is the checked Git tree from an xpec's most recent pass result.

**evaluator thread** is an ephemeral evaluator interaction context with no persisted history.

**turn** is one request to an evaluator agent and the evaluator agent's returned message.

**interrogation** is the sequence of turns performed during the evaluation of one xpec with `to: agent`.

---

**scope** is a Git pathspec list that defines a file subset within a Git-tracked tree.

**scoped tree** is the logical Git tree induced by applying a scope to a Git-tracked tree. It does not have to exist as a Git tree object.

**scoped tree OID** is the repository-native object ID that a scoped tree would have if stored as a Git tree object.
It uses the repository's object hash algorithm; it is not a custom digest of rendered metadata.
Canon may reuse the OID when Git already has it; otherwise canon serializes and hashes a synthetic tree object.

**q-scope** (**question scope**) is a scope complete for a question: if files outside the q-scope change while files inside it stay the same, the correct answer to the question should not change.

**q-scope suggestion** is an evaluator-provided scope claiming to be a narrow scope sufficient to answer the question.
It may or may not be a valid q-scope.

**visible scope** is a q-scope plus ignore patterns applied as exclusions.

**visible tree** is the scoped tree induced by the visible scope.

---

**new pass** is a check-run classification for an xpec whose current result is `pass` and for which no persisted `pass` result existed when the check run started.

**regression** is a check-run classification for an xpec whose current result is `fail` and for which a persisted `pass` result existed when the check run started.

---

**CANON_STATE_DIR** is the root directory for all canon-owned non-temporary persistent state.
It is set by the `CANON_STATE_DIR` environment variable. If the variable is unset, it defaults to `$(git rev-parse --git-path canon)`.
Every canon command stores canon-owned non-temporary persistent state only under `CANON_STATE_DIR`.

**XPECS_DIR** is `${CANON_STATE_DIR}/xpecs`.
