# CLI / commands / `canon ask`

```python
log = import(ref="#log")
setup_logging = import(ref="#setup_logging")
evaluate = import(ref="#evaluate")

def canon_ask(question):
    setup_logging()
    log.on('xpec.evaluation.start', _on_evaluation_start)
    log.on('xpec.evaluation.heartbeat', import(ref="#print_timeline_marker"))
    log.on('xpec.evaluation.finish', _on_evaluation_finish)
    log.on('ask.finish', import(ref="#print_token_usage"))
    try:
        evaluate(Xpec(to=AGENT, q=question, a=''))
    finally:
        log.info('ask.finish', ...)

def _on_evaluation_start(event):
    xpec = get_xpec_by_id(event['id'])
    print(xpec.short_id, end='', flush=True)

def _on_evaluation_finish(event):
    xpec = get_xpec_by_id(event['id'])
    print()  # end the short-ID/timeline line before printing details
    if (diff_from_oid := xpec.diff_from_oid) is not None:
        short_diff_from_oid = ...  # Git abbreviation of diff_from_oid
        print('diff-from:', short_diff_from_oid, f"({xpec.diff_from})")
    if (error := event.get('error')) is not None:
        print('error:', error)
    if (observed := event.get('observed')) is not None:
        assert error is None
        print('observed:', observed)
    if (evidence := event.get('evidence')) is not None:
        print('evidence:', evidence)
    if (q_scope_suggestion := event.get('qScopeSuggestion')) is not None:
        print('q-scope-suggestion:', compact_json(q_scope_suggestion))
    stdout.flush()
```

## Persistence

The only persistent state `canon ask` may write is runtime logs.
