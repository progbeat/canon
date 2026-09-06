# cli / commands / `canon ask`

The command installs logging handlers for the evaluation output and token usage specified below.

```python
evaluate = import(ref="#evaluate")

def canon_ask(question):
    try:
        evaluate(Xpec(to=AGENT, q=question, a=''))
    finally:
        emit_token_usage()
```

The only persistent state `canon ask` may write is runtime logs.
