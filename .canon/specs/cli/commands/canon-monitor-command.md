# CLI / commands / `canon monitor`

```sh
$ canon monitor --help
Monitor the current or most recent `canon check`.

Usage: canon monitor

Options:
  -h, --help  Print help
```

*The `canon monitor --help` output may differ in wording, wrapping, spacing, and option order while preserving the same command usage and options.*

## Interface

`canon monitor` uses a fullscreen session and the global TUI stylesheet. In this component layout, lowercase elements are slots or pages and `{...}` binds state:

```xml
<FrameLayout minWidth="24" minHeight="6"
             fallback="Terminal is too small (minimum 24×6).">
  <header>
    <ProgressRow count="{passed} / {collected}"
                 completed="{passed + failed}" total="{collected}"
                 duration="{runDuration}" />
  </header>

  <content>
    <PageView active="{page}">
      <page name="xpecs">
        <ListViewport items="{navigableSequence}" selected="{selectedXpec}">
          <Scrollbar />
          <OverviewRuler markers="{overviewMarkers}" />
        </ListViewport>
      </page>
      <page name="keys">
        <KeysPage shortcuts="{inputMap}" paddingTop="1" />
      </page>
    </PageView>
  </content>

  <footer>
    <HintBar />
  </footer>
</FrameLayout>
```

Each xpec row represents one checked expectation.
A failure-marked xpec is failed or is pending after a previous failure.
The display sequence follows the recorded `check.start.xpecs` array.
The navigable sequence is its suffix starting at zero-based index `max(0, C - floor(V / 2))`, where `C` is the leading cached-xpec count and `V` is the xpec viewport height.
Selection and scrolling are restricted to this suffix; numbering retains display-sequence positions.

The progress row is the sole pass-count display.

Each primary list row has this cell order:

```text
[number gutter] <marker> <short-ID field>  <time or pending-question field>  <ticker>
```

The marker is `✓`, `×`, `▷`, or `○` for passed, failed, active, or pending respectively.
The time field is empty before an xpec starts, contains its duration after it starts, and contains `cached` for a cached pass.
The ticker is formatted by the monitor from the xpec's latest received `changedFileCount`: `1 changed file` for one and `<count> changed files` otherwise, including zero.
This count is the non-negative number of diff-affected files in the current visible scope, available only for Git-backed evaluations.
It is absent until a count is received and remains visible after evaluation finishes.

The selected xpec's inline field blocks appear directly below its row, with their allocated areas starting at its marker column, and contain available fields in this order:

```yaml
<question>
scope: <scope>
expected: <expected>
error: <error>
observed: <observed>
evidence: <evidence>
```

Fields are shown when available; `error` takes the place of `observed`.
Without question metadata, question shows the full ID when available and `(metadata unavailable)` otherwise, while expected shows `(metadata unavailable)`.

Expanded question and evidence use wrapped text.
Question has no additional content indentation; evidence uses `evidence: ` as its first-row label and two cells of additional continuation indentation.
Expected, error, and observed always occupy one row.

Scope is `scope: full` for full project scope, otherwise `scope: N file(s)` using the latest received `qScopeFileCount`, singular only for one.
The paths come from `qScope`, the current q-scope as repository-relative path strings; `qScopeFileCount` is the non-negative number of distinct files they select in the checked tree, available only for Git-backed evaluations.
Expanded non-full scope appends its path forest with two cells of indentation; the summary is not a root.

`overviewMarkers` contains every active and failure-marked xpec in the navigable sequence, in display order, with row positions relative to the start of that sequence.

### Styling

Application meaning is mapped to global classes; other parts retain their shared component styles:

| Content or state | Classes |
| --- | --- |
| Passed marker and short ID | `success` |
| Cached marker and short ID | `success-muted` |
| Failed marker and short ID | `error` |
| Active or interrupted marker / short ID | `warning` / `warning strong` |
| Pending after pass / failure / no previous result | `info` / `error` / `muted`; marker additionally `dim` |
| Duration | `muted` |
| `cached` and ticker | `muted dim` |
| Entire pending question, including generated markers | `subtle` |
| Error and observed values | `error` |
| Progress fill: active / failed / successful / interrupted | `busy` / `error` / `success` / `warning` |
| Interrupted progress duration | `warning` |
| Active / failure overview marker | `mark-warning` / `mark-error` |

## Behavior

### Status consumption

Before starting its sources, the monitor registers a `log.on` callback for each event below.
These callbacks update replayed run data; source observations and user input update the monitor's local state.
User input owns selection, expansion, and scroll state; elapsed durations are derived from event timestamps and the local clock.

| Event | Model update |
| --- | --- |
| `check.start` | Begin the run from `xpecs` and `collectedCount`; request metadata using `configPath` and `treeOid` |
| `xpec.evaluation.start` | Start the identified xpec and apply its evaluation fields |
| `xpec.evaluation.update` | Apply the identified xpec's changed evaluation fields |
| `xpec.evaluation.heartbeat` | Advance its last recorded `activity` and timestamp |
| `xpec.evaluation.finish` | Apply `status`, available `error`, `observed`, and `evidence`, and finish timestamp |
| `check.finish` | Apply the run's final `result` and finish timestamp |

`xpecs` lists reused passes in checked-tree order followed by Selected expectations in evaluation order.
Each entry carries its full `id`, pre-run `previousStatus` (`pass`, `fail`, or null), current `status` (`pending`, `active`, `pass`, or `fail`), and Boolean `cached` indicating whether the current result was taken from cache without a new evaluation.
`collectedCount` is the number of collected expectations.
`configPath` is repository-relative and `treeOid` identifies the checked tree.
Evaluation callbacks identify xpecs by full `id`; omitted evaluation fields retain their previous values.

An asynchronous task follows `runs/latest.jsonl`, passing complete records to `log.emit` unchanged in file order.
It reads from byte zero initially and on target replacement or truncation; otherwise it reads only appended bytes, retaining incomplete final lines.
For an already replayed target, rereading verifies and skips its previously delivered record prefix; a changed prefix is a status-log error.
At EOF it waits for data or target changes independently of UI input and rendering.

Sources resolve checked-tree metadata in-process against `treeOid`: full IDs, short IDs, questions, and expected answers in checked-tree order, plus directory classification for scope paths.
Until that metadata is available, use shortest unique prefixes of the logged full IDs as provisional row labels.
Their results update local metadata, source diagnostics, and interruption state directly, without logging events; fields not supplied by a result retain their values.
The resolved target path is null until a target can be opened.
Sources identify a new target before replaying its `check.start` and discard stale asynchronous results after replacement or truncation.
The monitor only replays existing events, generates no new events, installs no file-writing handlers, and cancels and awaits its sources on exit.

Runs remain visible until the next `check.start`, which resets run data and diagnostics but retains UI preferences.
Recoverable source errors replace footer hints with `<source>: <error>` until recovery, while input and retries continue; simultaneous errors prefer `metadata`, then `status log`, then `check.lock`.
Rendering may coalesce updates without dropping model updates.

### Run state and duration

Without a recorded finish, probe the existing check lock without creating it or waiting.
If available, drain complete events and confirm under the lock observation that the target and read position still describe the same unchanged file.
If still unfinished, set local `interruptedAfter = max(run start, last complete event timestamp)`.
Lock errors are diagnostics, not interruption; a recorded finish takes precedence.

The run is active until finished or detected interrupted; a finished run is failed if its final result is `fail` or any xpec failed, and successful otherwise.
For interruption, progress duration is `interrupted ≥<duration>`, an unfinished started xpec uses marker `?` and duration `≥<duration>`, and it has no active overview marker; unfinished xpecs are not assigned a fabricated result.

Durations use `humanize::compact_duration`, clamping negative differences to zero.
Run duration starts at the `check.start` timestamp; its endpoint is the `check.finish` timestamp, otherwise the interruption lower bound, otherwise now.
Xpec duration uses its start and finish timestamps; without a finish, its endpoint is the interruption lower bound when present and now otherwise.
The interruption lower bound stays fixed between events.
Pass and failure counts are the numbers of xpecs whose current `status` is `pass` and `fail`, respectively.

### Selection, details, and follow

Clicking any cell of another primary row selects it; clicking the selected row toggles details.
When details are open, changing selection transfers the block to the newly selected xpec without changing field expansion state.

Initially, follow, details, and numbering are on; only evidence is expanded.
Only a question or evidence containing a line break or exceeding its one-row value width, and a non-full scope, are expandable fields.
Numbered field bindings are stable across xpecs and affect only their assigned expandable field.
`0` collapses currently expandable fields without resetting the remembered state of absent or currently single-row fields.

Follow selects the active xpec, otherwise the last non-cached completed xpec, otherwise the first non-cached pending xpec.
When every xpec is cached, it selects the last xpec; an empty run has no selection.
Outside follow mode, status updates preserve the selected xpec by identity while it remains in the display sequence and otherwise use the follow-mode target.
After data or viewport-size changes, selection before the navigable sequence moves to its first xpec.
Only xpec navigation, clicking another xpec, or scrolling its viewport disables follow.

Follow continuously centers the selected block after data, details, or terminal-size changes; `z` centers it once without enabling follow.
Collapsing a field or the details block uses the list viewport's center-preserving collapse behavior.
Outside follow mode, navigating to an xpec, opening its details, or toggling numbering requests reveal rather than centering; expanding a field preserves scroll within its bounds.
`page` is initially `xpecs`; toggling the keys page switches it between `xpecs` and `keys`.
Opening the keys page resets its scroll to the top; closing it retains the xpec-view state, including any follow updates received meanwhile.

### Input

`inputMap` is the complete input and keys-page vocabulary:

| Group | Keys | Description |
| --- | --- | --- |
| Navigation | `j` / `↓` | Next xpec |
| Navigation | `k` / `↑` | Previous xpec |
| Navigation | `g` / `Home` | First xpec |
| Navigation | `G` / `End` | Last xpec |
| Scrolling | `e` | Scroll down one line |
| Scrolling | `y` | Scroll up one line |
| Scrolling | `d` | Scroll down half a page |
| Scrolling | `u` | Scroll up half a page |
| Scrolling | `z` | Center xpec |
| Actions | `↩` / `Space` | Toggle details |
| Actions | `→` / `l` | Open details |
| Actions | `←` / `h` | Close details |
| Actions | `f` | Follow current xpec |
| Actions | `?` | Toggle keys page |
| Actions | `q` | Quit |
| Display | `n` | Toggle line numbers |
| Details | `1` | Toggle question |
| Details | `2` | Toggle scope |
| Details | `3` | Toggle evidence |
| Details | `0` | Collapse expanded fields |
| Mouse | Click row | Focus xpec / toggle details |
| Mouse | Click field | Toggle field |
| Mouse | Click footer | Activate action |
| Mouse | Click scrollbar | Scroll to position |
| Mouse | Wheel | Scroll |

Navigation traverses the navigable sequence; on the keys page, next/previous scroll one row and first/last go to the page boundaries.
Scrolling moves only the visible viewport, never selection, and has no effect when it fits. Wheel steps are three rows.
Half-page steps use `max(1, floor(scrolling_viewport_height / 2))`, excluding the keys page's fixed blank row.
`q` exits from every page and is the only exit key.
On the keys page, only navigation, scrolling except `z`, wheel, scrollbar/footer clicks, `?`, and `q` act; other bindings are documentation.

### Adaptive layout

Collapsed detail values and pending questions use ellipsized inline values; short IDs, times, and tickers use clipped inline values.

The list columns are short ID (left-aligned) and time (right-aligned), each capped at eight cells.

A pending question begins at the time-field position and uses otherwise available cells before the ticker.
It is omitted when the ticker requires that space and when the pending xpec is selected with details open.

The ticker is right-aligned when it fits with at least two cells after preceding content; otherwise it begins after those two cells and is clipped at the right edge.

Footer left groups, in order: `f follow`; `↩/→ details` or `↩/← close details`; `<number> <field>` hints and `0 collapse`; `n numbers`.
The follow hint is absent while follow mode is active.
Numeric hints appear only for open details and keys with a visible effect; `0` appears only while an expandable field is expanded.
Persistent right groups are `? keys` and `q quit`.
The keys-page footer instead has `? close` on the left and `q quit` on the right.
