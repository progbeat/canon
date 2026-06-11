# Glossary

**expectation** is the basic unit of the canon: a formalized human expectation expressed as a question and expected-answer pair.

**ID** is a 20-character base62 hash derived from two separate input fields: the rendered expectation question and a deterministic hash of the expectation instructions.

**short ID** is the shortest prefix of an expectation's **ID** that uniquely identifies that expectation among the collected expectations.

**scope** is a Git pathspec list that defines a file subset within a Git-tracked tree.

**scoped tree** is the logical Git tree induced by applying a scope to a Git-tracked tree. It does not have to exist as a Git tree object.

**scoped tree OID** is the repository-native object ID that a scoped tree would have if stored as a Git tree object.
It uses the repository's object hash algorithm; it is not a custom digest of rendered metadata.
Canon may reuse the OID when Git already has it; otherwise canon serializes and hashes a synthetic tree object.

**q-scope** (**question scope**) is a scope complete for a question: if files outside the q-scope change while files inside it stay the same, the correct answer to the question should not change.

**q-scope suggestion** is an evaluator-provided scope claiming to be a narrow scope sufficient to answer the question.
It may or may not be a valid q-scope.

**visible scope** is the scope applied to a Git-tracked tree for an evaluator interrogation.
It is formed from the latest q-scope for the expectation, or full project scope when no verified q-scope exists. Configured ignore patterns are normalized as project-relative pathspec items, converted to excluding pathspec items, and applied last.

**visible tree** is the scoped tree induced by the visible scope.

**evidence** is evaluator-provided text that directly justifies an answer.

**expectation instructions** are the resolved `instructions` value for an expectation, or empty text when none is configured.

**evaluator thread** is an ephemeral, reusable evaluator interaction context with no persisted history. All interrogations on one thread must use the same evaluator model, the same visible tree, and the same expectation instructions; an interrogation with a different evaluator model, visible tree, or expectation instructions must use a different thread.
