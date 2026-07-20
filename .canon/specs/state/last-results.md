# Last Results

Each xpec may have up to two status-specific files: `last-pass.json` and `last-fail.json`.

The updated status-specific file is chosen from the final evaluation status:

- `last-pass.json` for `PASS`;
- `last-fail.json` for `FAIL`, including an evaluation response with `error` or a failure while acquiring an evaluation response.

When a **same-tree result** is reused, `last-pass.json` is updated.

`$XPECS_DIR/$ID/last.json` stores the same JSON object as the most recently updated status-specific file. When possible, it is a hardlink to that file; otherwise it is a copy.

Each `last-<status>.json` file stores a JSON object with this schema:

- `responseTimestamp` is required and uses ISO 8601 format.
  It records when `response` was produced.
- `updatedTimestamp` is required and uses ISO 8601 format.
  It records when the file was written.
- `status` is required and is `pass` or `fail`.
- `response` is required.
- `qScope` is required when the xpec was evaluated against a Git tree.
  It is the q-scope used to form the visible scope for the evaluation that produced `response`.
- `visibleScope` is required when the xpec was evaluated against a Git tree.
  When present, it is the visible scope used for the evaluation that produced `response`.
- `diffFrom` is required when `response` came from a Git-backed evaluator interrogation whose context included a prompt-rendered Git diff.
  It records the configured `diff-from` value used by that interrogation.
- `diffFromTreeOid` is required when `diffFrom` is present.
  It records the full resolved `diff-from` tree OID used by that interrogation.
- `checkedTreeOid` is required when the xpec was evaluated against a Git tree; it is omitted otherwise.
  It records the current checked tree OID and is not inherited from the reused pass result.
- `visibleTreeOid` is required when `status` is `pass` and the xpec was evaluated against a Git tree; it is omitted otherwise.
