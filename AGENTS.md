# AGENTS.md

- To run `canon check`, use `cargo run -- check` from the project root.
- After a successful commit, rebuild and refresh the installed `canon` binary available on PATH.
- Treat tokens as a scarce resource. Avoid increasing token usage unless the correctness benefit justifies it, and prefer designs that preserve or reduce the model work needed to answer canon questions correctly.
- If the evaluator returns a valid answer that does not match the expected answer, never try to influence the answer through developer instructions.
- Optimize evaluator developer instructions only to reduce token usage or to fix errors such as unparseable answers.
- Keep the evaluator agent’s developer instructions concise.
- Treat any codex_app_server ERROR or permission-config warning during canon check as a blocker, even if the command exits successfully.
- For reference, Codex GitHub: https://github.com/openai/codex

## Canon

The canon is the single source of truth for how the project should work.
Your job is to protect the canon and enforce it in the project.

To check the canon against the staged tree, run `canon check` with escalation.

### Change Constraints

If a request contradicts the canon or the canon is internally inconsistent, stop, show the human evidence based only on files under `.canon/`, and ask them to update the canon first.

Do not edit files under `.canon/` proactively. Edit them only when a human explicitly insists.

Before editing files, first read the relevant expectations under `.canon/` for the requested change. Do not start editing until you know which canon behavior must be preserved.

### Canon Enforcement

Always stage your edits before running `canon check`, because it does not check unstaged changes.

When `canon` writes an instruction, execute it. If the instruction says to commit, commit immediately.

When you are already making project changes and the canon is violated, whether detected by `canon check` or after a human updates the canon, proactively fix the implementation to match the canon without waiting for a separate human command.
Continue until there are no remaining issues that you are allowed and able to fix. When a fix causes a regression, improve the readability of the fragile logic before retrying.

Do not take `canon check` evidence on trust.
Before acting on a result, verify that the evidence actually supports the observed answer and answers the expectation question.
If `canon check` gives a wrong answer, unsupported evidence, or evidence that is irrelevant to the question while the project satisfies the expectation, treat that as a readability issue.

Follow canon terminology in implementation code and documentation.
If project terminology drifts away from terms defined in the canon, treat that as a readability issue.

When improving readability, use comments where they help, but prefer making the code self-explanatory through clearer naming, better structure, and focused refactoring.

### Committing

Never commit `.canon/` changes.
Before committing, run `git diff --cached --quiet -- .canon/`; if it exits `1`, stop and ask a human to handle them.

Never commit with `-n` or `--no-verify`.

Before creating a commit, run `canon check` with escalation and no expectation filters.
