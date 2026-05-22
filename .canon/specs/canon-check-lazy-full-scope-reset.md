# `canon check` Lazy Full-Scope Reset

At the end of a `canon check` invocation, once final token usage data is
available, the following lazy full-scope reset policy is applied to
non-selected expectations:

```
def stochastic_round(x):
    n = floor(x)
    p = x - n
    return n + int(random() < p)

def lazy_full_scope_reset(num_evaluated_expectations, skipped_expectations):
    """
    num_evaluated_expectations: Number of expectations processed by the evaluator agent.
    skipped_expectations: Non-selected expectations.
    """
    candidates = [e for e in skipped_expectations if e.scope != ["."]]
    num_to_reset = min(
        stochastic_round(num_evaluated_expectations / 128),
        len(candidates),
    )
    expectations_to_reset = random.sample(candidates, num_to_reset)
    for expectation in expectations_to_reset:
        schedule_at_next_canon_check_run(set_scope, expectation, ["."])
    # Takes effect at the beginning of the next `canon check` invocation.
```

This prevents long-lived narrowed scopes from missing rare cases where changes
outside the last known expectation scope could affect the expectation's answer.
