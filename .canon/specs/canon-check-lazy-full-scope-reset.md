# `canon check` Lazy Full-Scope Reset

At the end of a `canon check` invocation, the following lazy full-scope reset policy is applied to cached expectations:

```
def stochastic_round(x):
    n = floor(x)
    p = x - n
    return n + int(random() < p)

def lazy_full_scope_reset(num_evaluated_expectations, cached_expectations):
    candidates = [e for e in cached_expectations if e.q_scope != ["."] and e.result == "pass"]
    num_to_reset = min(
        stochastic_round(num_evaluated_expectations / 128),
        len(candidates),
    )
    expectations_to_reset = random.sample(candidates, num_to_reset)
    for expectation in expectations_to_reset:
        schedule_at_next_canon_check_run(reset_to_full_project_q_scope, expectation)
    # Takes effect at the beginning of the next `canon check` invocation.
```

This prevents long-lived verified q-scopes from missing rare cases where changes outside the latest verified q-scope could affect the expectation's answer.
