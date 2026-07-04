# AGENTS.md

- Run repeated `canon check` invocations with `cargo run -- check` by default. For roughly 1/10 runs, rebuild `canon:local` first and run `CANON_DOCKER_IMAGE=canon:local .canon/docker/scripts/canon check` instead.
- After a successful commit, rebuild and refresh the installed `canon` binary available on PATH.
- Treat tokens as a scarce resource. Avoid increasing token usage unless the correctness benefit justifies it, and prefer designs that preserve or reduce the model work needed to answer canon questions correctly.
- If the evaluator returns a valid answer that does not match the expected answer, never try to influence the answer through developer instructions.
- Optimize evaluator developer instructions only to reduce token usage or to fix errors such as unparseable answers.
- Keep the evaluator agent’s developer instructions concise.
- For every test declaration and every assert invocation, the behavior checked on that execution path must logically follow from the canon. Add a source-comment marker `xpec: <shortID>[,<shortID>...]`, preferring the same line over the immediately preceding non-blank line. A marker on a test declaration covers assert invocations in that test unless an assert has its own marker.
- Treat any codex_app_server ERROR or permission-config warning during canon check as a blocker, even if the command exits successfully.
- For reference, Codex GitHub: https://github.com/openai/codex

## AI Docs

Read `docs/ai/README.md` for the AI docs purpose and usage.

Use `docs/ai/**` for compact notes that reduce future confusion: recurring failures, reliable fixes, project gotchas, navigation tips, canon pain points, questionable canon decisions, implementation concerns, and improvement ideas.

You may edit `docs/ai/**` when useful.

Keep `docs/ai/**` small. Do not store raw logs, long outputs, transcripts, duplicate notes, or stale complaints.

## Canon

The canon is the single source of truth for how the project should work.
Your job is to **protect** the canon and **enforce** it in the project.

To check the canon against the staged tree, run `canon check` with escalation.

When running canon check, **do not pass options** unless the human explicitly requests them.

### Change Constraints

**Never ever make unrequested changes unless they directly improve the project's compliance with the canon.**

If a request appears to contradict the canon, or if the canon appears internally inconsistent, use `$canon-conflict` before reporting a conflict to the human.

Do not edit files under `.canon/` proactively. Edit them only when a human explicitly insists.

**Every edit may potentially violate the canon**, so before editing project files, **always** run `canon show -- <PATHSPEC>...` for the planned edit paths to identify the relevant expectations under `.canon/`. Do not start editing until you can explain why the planned change is compatible with those expectations.

### Canon Enforcement

Always stage your edits before running `canon check`, because it does not check unstaged changes.

When `canon` writes an instruction, execute it. If the instruction says to commit, commit immediately.

When you are already making project changes and the canon is violated, whether detected by `canon check` or after a human updates the canon, proactively fix the implementation to match the canon without waiting for a separate human command.
Continue until there are no remaining issues that you are allowed and able to fix. When a fix causes a regression, improve the readability of the fragile logic before retrying.

When a test fails, compare the behavior asserted by the test with the canon.
If the behavior follows from the canon, fix the implementation.
If the behavior contradicts the canon, delete the test.

**Do not take `canon check` evidence on trust.**
Before acting on a result, verify that the evidence actually supports the observed answer and answers the expectation question.
If `canon check` gives a wrong answer, unsupported evidence, or evidence that is irrelevant to the question while the project satisfies the expectation, **treat that as a readability issue**.

Follow canon terminology in implementation code and documentation.
If project terminology drifts away from terms defined in the canon, treat that as a readability issue.

When improving readability, use comments where they help, but prefer making the code self-explanatory through clearer naming, better structure, and focused refactoring.

### Committing

Never commit `.canon/` changes.
Before committing, run `git diff --cached --quiet -- .canon/`; if it exits `1`, stop and ask a human to handle them.

Never commit with `-n` or `--no-verify`.

Before creating a commit, run `canon check` with escalation and no expectation filters.
