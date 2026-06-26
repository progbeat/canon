# Evaluator stalls and log gaps

## Request

Review whether canon should add stronger evaluator-result validation and more
detailed turn diagnostics. Recent runs stalled because the repair agent received
failures whose evidence did not prove the observed answer, while long evaluator
turns had little useful log detail between request and response.

## Problems to address

- Reject or retry evaluator evidence that argues from Git diff changedness, such
  as "the touched code only changes..." or "the diff does not add...". The diff
  is navigation context, not proof.
- Reject or retry evaluator evidence that cites project paths absent from the
  visible project.
- Add a hard wall-clock timeout for evaluator turns, separate from the current
  no-progress/idle timeout. App-server activity or token updates should not let
  a simple turn run for many minutes without a failure or fallback.
- Log bounded turn progress between `agent.request` and `agent.response`:
  elapsed time, last app-server method, turn id when known, token totals, and
  whether activity is token-only or actual answer/tool progress.
- Log enough visibility context on `thread.start` to reproduce "only template
  output is visible" failures: session cwd, template output dir, visible scope,
  visible tree oid, visible file count, and a bounded sample/count of visible
  files.

## Why

Without these checks, the repair agent can chase unsupported failures instead
of fixing real implementation gaps.
