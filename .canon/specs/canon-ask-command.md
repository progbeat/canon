# `canon ask` Command

`canon ask <QUESTION>` asks one ad-hoc question.

`canon ask` constructs a temporary in-memory xpec whose question is `<QUESTION>` and whose expected answer is empty text.

The temporary xpec follows the same evaluator path as a `canon check` xpec.

For `canon ask`, the empty expected answer means the evaluator response is reported without deriving a result.

Stdout starts with `<progress timeline>` on its own line.

Then stdout prints the evaluator response.

For an answer, the response lines are:

```
Observed: <escaped observed answer>
Evidence: <escaped evidence>
```

For an evaluator error, the response lines are:

```
Error: <escaped error>
Evidence: <escaped evidence>
```

If a q-scope suggestion is available, stdout appends `Suggested q-scope: <compact JSON array>` as the final line.

The line is omitted when no q-scope suggestion is available.

Text fields are escaped the same way as `canon check` stdout.

After evaluator work finishes, stderr contains exactly one token usage line in the same format as `canon check`.

`canon ask` always asks the evaluator.

The only persistent state `canon ask` may write is runtime logs.
