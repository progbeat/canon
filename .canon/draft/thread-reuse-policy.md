# Thread Reuse Policy

Since cached tokens are cheap, `canon check` reuses the evaluator thread across
expectations. The benefit of carried-over tokens appears to plateau early, while
the cost of carrying more tokens keeps increasing. To capture that tradeoff,
`canon check` keeps carried-over tokens within a target range by rolling back
turns that would move the thread outside it.

```
def carryover_tokens(turn):
    last_usage = turn.last_usage
    return last_usage.input_tokens + last_usage.output_tokens

def thread_reuse_policy(thread):
    prev_turn, curr_turn = thread.turns[-2:]
    if carryover_tokens(prev_turn) >= MIN or carryover_tokens(curr_turn) > MAX:
        thread.rollback()  # kind of thread.turns.pop()
```

| Parameter | Default | Meaning |
| --- | --- | --- |
| `canon.threadReuse.carryoverTokenTarget` | `10000,30000` | Target range `MIN,MAX` for carried-over tokens. |
