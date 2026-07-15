# `canon ask` Command

```python
def canon_ask(question):
    try:
        evaluate(Xpec(to=AGENT, q=question, a=''))
    finally:
        emit_token_usage()
```

The only persistent state `canon ask` may write is runtime logs.
