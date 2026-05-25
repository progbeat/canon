# Cache

`CACHE_DIR` is `${CANON_STATE_DIR}/cache`.

Each expectation has an ID. The ID is a 20-character base62 hash derived from the
expectation question and expected answer.

`canon check` stores per-expectation data (e.g. answer history) under `$CACHE_DIR/$ID`.

## Answer History

Answer history files use JSON Lines format and store only valid answers (i.e., `pass` or `fail`). Each history record contains at least these fields in order:

```text
timestamp
result
observed
evidence
scope
visibleTreeOid
```

`result` is either `pass` or `fail`.

`observed` is the evaluator answer that is compared with the expected answer. History records are written only for correct or incorrect answers.

`timestamp` is UTC and records when the history record is produced.

`scope` is either `["."]` or a list of normalized repository-relative paths with redundant child paths removed when a parent directory path already covers them.

`visibleTreeOid` is the Git-compatible OID of the evaluator-visible tree: the tracked Git entries that are visible to the evaluator after applying enforced scope and ignore rules. The OID uses the repository's object hash algorithm; it is not a custom digest of
rendered metadata. As an optimization, canon reuses the required OID when Git already has it. Otherwise, canon serializes and hashes a synthetic tree object.

`canon check` compacts a history file with approximately a 1-in-16 chance after
appending a record. Compaction keeps at least the latest 8 valid JSON object
records.
