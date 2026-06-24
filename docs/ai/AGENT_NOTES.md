# Agent notes

- Before editing project files, run `canon show -- <PATHSPEC>...` for the exact planned paths and use those expectations to constrain the edit.
- The canon is the source of truth. If expectations seem contradictory, first look for an interpretation where they are compatible; stop only when files under `.canon/` prove a real contradiction.
- Treat `canon check` evidence as feedback, not truth. Verify that it supports the observed answer and answers the expectation before changing behavior.
- If an evaluator result has `error: "InvalidQuestion"`, either tell the human or fix the implementation/readability issue that made the question invalid.
