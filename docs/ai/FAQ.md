# AI FAQ

- `canon check` must run against staged changes, so stage intended edits before the final check.
- If `canon check` reports unsupported evidence while the code satisfies the expectation, treat it as a readability issue and clarify the relevant source or docs.
- Do not commit `.canon/` changes from an agent-authored commit; ask the human to own canon updates.
- Put human-facing AI handoff notes in `docs/ai/HUMAN_HANDOFF.md`, then explicitly ask the human to open that file. Do not create root-level docs for AI notes.
