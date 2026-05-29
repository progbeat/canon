# Glossary

**expectation** is the basic unit of the canon: a formalized human expectation expressed as a question and expected-answer pair.

**scope** is a Git pathspec list that defines a file subset within a Git-tracked tree.

**scoped tree** is the logical Git tree induced by applying a scope to a Git-tracked tree. It does not have to exist as a Git tree object.

**q-scope** (**question scope**) is a scope complete for a question: if files outside the q-scope change while files inside it stay the same, the correct answer to the question should not change.

**q-scope suggestion** is an evaluator-provided scope claiming to be a narrow scope sufficient to answer the question.
It may or may not be a valid q-scope.

**visible scope** is the scope applied to a Git-tracked tree for an evaluator interrogation.
It is formed from the latest q-scope for the expectation, or full project scope when no verified q-scope exists. Configured ignore patterns are normalized as project-relative pathspec items, converted to excluding pathspec items, and applied last.

**visible tree** is the scoped tree induced by the visible scope.

**evidence** is evaluator-provided text that directly justifies an answer.

**evaluator thread** is an ephemeral, reusable evaluator interaction context with no persisted history. All interrogations on one thread must use the same evaluator model and the same visible tree; an interrogation with a different evaluator model or visible tree must use a different thread.
