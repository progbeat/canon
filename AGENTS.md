# AGENTS.md

- Use the $canon-warden skill when editing files.
- To run `canon check`, use `cargo run -- check` from the project root.
- After a successful commit, rebuild and refresh the installed `canon` binary available on PATH.
- Treat tokens as a scarce resource. Avoid increasing token usage unless the correctness benefit justifies it, and prefer designs that preserve or reduce the model work needed to answer canon questions correctly.
- If the evaluator returns a valid answer that does not match the expected answer, never try to influence the answer through developer instructions.
- Optimize evaluator developer instructions only to reduce token usage or to fix errors such as unparseable answers.
- Keep the evaluator agent’s developer instructions concise.
- For reference, Codex GitHub: https://github.com/openai/codex
