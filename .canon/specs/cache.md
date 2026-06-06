# Cache

`CACHE_DIR` is `${CANON_STATE_DIR}/cache`.

`canon check` stores per-expectation data (e.g. answer history) under `$CACHE_DIR/$ID`, where `ID` is the expectation's ID.

## Answer History

Answer history files use JSON Lines format and store only schema-valid responses with `answer`. Error responses are not answer history records. Each history record contains at least these fields in order:

```text
timestamp
observed
evidence
qScope
visibleTreeOid
```

`observed` is the evaluator response's `answer` value. The current result is derived by comparing `observed` with the current expected answer.

`timestamp` is UTC and records when the history record is produced.

`qScope` stores the q-scope used to form the visible tree for this history record.

`visibleTreeOid` stores the scoped tree OID of the visible tree.

`canon check` compacts history files with approximately a 1-in-16 chance after appending a record. Compaction keeps at least the latest 8 valid JSON object records.
