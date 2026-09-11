# cli / common

## Logging Setup

```python
log = import(ref="#log")

@ref("#setup_logging")
def setup_logging():
    if LOGS_MAX_SIZE == 0:
        return
    log.on('*', log.RotatingFileHandler(LOGS_DIR, max_bytes=LOGS_MAX_SIZE))
```

The subscription retains the handler for the process lifetime.

## Timeline Handler

```python
@ref("#print_timeline_marker")
def print_timeline_marker(event):
    print(event['marker'], end='', flush=True)
```

## Token Usage Handler

**Command token usage** is an object containing the command's final aggregate counts: `totalTokens`, `inputTokens`, `cachedInputTokens`, `outputTokens`, and `reasoningOutputTokens`, each a non-negative integer, plus their reference token cost as `referenceCost`.
All fields are zero when usage is unavailable.
The command's finish event carries this object as `tokenUsage`.

```python
@ref("#print_token_usage")
def print_token_usage(event):
    usage = event['tokenUsage']
    print(
        f"token-usage: ref-cost={usage['referenceCost']:.2f}$ total={usage['totalTokens']} "
        f"input={usage['inputTokens']} ({usage['cachedInputTokens']} cached) "
        f"output={usage['outputTokens']} (reasoning {usage['reasoningOutputTokens']})",
        file=stderr, flush=True,
    )
```
