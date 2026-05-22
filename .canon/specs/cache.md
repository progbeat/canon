# Cache

`CACHE_DIR` is `${CANON_STATE_DIR}/cache`.

Each expectation has an ID. The ID is a 20-character base62 hash derived from the
expectation question and expected answer.

`canon check` stores per-expectation data (e.g. answer history) under `$CACHE_DIR/$ID`.

Answer history files use JSON Lines format. Each non-empty line is one complete JSON
object.

Each history record contains at least these fields in order:

```text
timestamp
result
observed
evidence
scope
scopeTreeOid
```

`result` is either `pass` or `fail`.

`observed` is the evaluator answer that is compared with the expected answer.
History records are written only for correct or incorrect answers. Non-answer
states such as `idk` and `malformed` are not written to history.

`timestamp` is UTC and records when the history record is produced.

`scope` is the scope for the cached result. It is either `["."]` or a
lexicographically sorted, duplicate-free list of normalized repository-relative
paths with redundant child paths removed when a parent directory path already
covers them.

`scopeTreeOid` is the Git-compatible object ID of the scoped tree: the subset of
tracked Git entries covered by `scope`, with repository-relative paths, modes,
object IDs, and tree structure preserved from the Git state being checked.

Canon reuses existing Git object IDs for files and fully covered directories,
and only serializes/hashes synthetic tree objects for partially covered
directories. The object ID uses the repository's object hash algorithm;
it is not a custom digest of rendered metadata.

`canon check` compacts a history file with approximately a 1-in-15 chance after
appending a record. Compaction keeps at least the latest five valid JSON object
records.

When looking up a cached result, `canon check` scans the answer history file from
newest-to-oldest and selects the first record whose `scopeTreeOid` matches the
current `scopeTreeOid` for that record's `scope` in the Git state being checked.
