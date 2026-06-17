# Last Results

Each xpec may have up to three status-specific files: `last-pass.json`, `last-fail.json`, and `last-error.json`.

The updated status-specific file is chosen from the final `response`:

- `last-pass.json` when `response.answer` matches the xpec's expected answer;
- `last-fail.json` when `response.answer` exists and does not match the xpec's expected answer;
- `last-error.json` when evaluation does not produce a usable `answer`, including evaluator responses with `error` and failures while attempting evaluator work.

When a **same-tree result** reuses a pass or fail, the corresponding files are refreshed.

`$XPECS_DIR/$ID/last.json` stores the same JSON object as the most recently updated status-specific file. When possible, it is a hardlink to that file; otherwise it is a copy.

Each `last-<status>.json` file stores a JSON object with this schema:

- `responseTimestamp` is required and uses ISO 8601 format.
  It records when `response` was produced.
- `updatedTimestamp` is required and uses ISO 8601 format.
  It records when the file was written.
- `status` is required and is `pass`, `fail`, or `error`.
- `response` is required.
- `qScope` is required.
  It is the q-scope used to form the visible scope for the evaluator work that produced `response`.
- `visibleScope` is required.
  It is the visible scope used for the evaluator work that produced `response`.
- `checkedTreeOid` is required when `status` is `pass` and omitted otherwise.
  It records the current checked tree OID and is not inherited from the reused pass result.
- `visibleTreeOid` is required when `status` is `pass` or `fail` and omitted when `status` is `error`.

`response` is the normalized final evaluator response. If evaluator work fails before producing a schema-valid evaluator response, `response` is the normalized error response recorded for that failure.
