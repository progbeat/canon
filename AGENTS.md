# AGENTS.md

- Use the $canon-warden skill.
- Run `canon check` through `cargo run -- check` with escalation.
- Treat tokens as a scarce resource. Avoid increasing token usage unless the correctness benefit justifies it, and prefer designs that preserve or reduce the amount of model work needed to answer canon questions correctly.
- If there is no error, but the evaluator’s answer does not match the expected answer, never try to influence the answer through developer instructions.
- You may optimize developer instructions only to reduce token usage or to fix errors such as unparseable answers.
- Keep the evaluator agent’s developer instructions concise.
- Codex GitHub: https://github.com/openai/codex
