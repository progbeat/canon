# `canon status` Command

```sh
$ canon status --help
Show the current or most recent `canon check` status.

Usage: canon status [OPTIONS]

Options:
      --watch  Refresh the display while a check is running
  -h, --help   Print help
```

*The `canon status --help` output may differ in wording, wrapping, spacing, and option order while preserving the same command usage and options.*

## Check execution

Status publication and check serialization require a resolvable `CANON_STATE_DIR`.
Only `canon check` acquires the operating-system-managed exclusive lock, doing so before evaluation and waiting for its current holder when necessary.
The operating system releases it when the holder exits.

## Append-only status log

Only Git-backed `canon check` writes status logs, creating one append-only **status log** per invocation using this layout:

```text
runs/
  <timestamp>-<unique>.jsonl
  latest.jsonl -> <timestamp>-<unique>.jsonl
```

Each line is one complete structured JSON event that is flushed when appended.

Status logs contain only records matching this schema; angle brackets denote JSON values, square brackets denote optional fields, and object key order is irrelevant:

```text
timestamp          := <non-negative Unix-millisecond integer>
status             := "pass" | "fail"
previousStatus     := status | null
ticker             := [<string>,...]
initial            := {"event":"initial","timestamp":timestamp,"selected":[{"id":<full-ID string>,"previousStatus":previousStatus},...],"collectedCount":<non-negative integer>,"reusedPassCount":<non-negative integer>,"configPath":<repository-relative string>,"treeOid":<string>}
evaluationStart    := {"event":"evaluation-start","timestamp":timestamp,"id":<full-ID string>[,"ticker":ticker]}
evaluationUpdate   := {"event":"evaluation-update","timestamp":timestamp,"id":<full-ID string>[,"ticker":ticker]}
evaluationPass     := {"event":"evaluation-finish","timestamp":timestamp,"id":<full-ID string>,"status":"pass"}
evaluationFail     := {"event":"evaluation-finish","timestamp":timestamp,"id":<full-ID string>,"status":"fail"[,"observed":<string>][,"evidence":<string>]}
evaluationError    := {"event":"evaluation-finish","timestamp":timestamp,"id":<full-ID string>,"status":"fail","error":<string>[,"evidence":<string>]}
checkFinish        := {"event":"check-finish","timestamp":timestamp,"result":status}
```

A **ticker** is an evaluation's ordered list of auxiliary display strings.

`selected` is exactly the ordered **Selected** set, with `previousStatus` read before the run; `collectedCount` counts **Collected** expectations.
After `initial` is flushed, `latest.jsonl` is atomically replaced with a symlink whose relative target is exactly the new log file name.
Completed progress is `reusedPassCount` plus the number of `evaluationPass`, `evaluationFail`, and `evaluationError` records; its total is `collectedCount`.
The run is running before `checkFinish`, whose `result` is its explicit final result.

`canon check` shows the number of diff-affected files in each evaluator agent turn's visible scope in the evaluation's ticker as `1 changed file` or `<count> changed files`.

Old status logs are removed automatically according to a bounded retention policy.
Cleanup never removes the file currently being written or the target of `runs/latest.jsonl`.

## Status display

`canon status` opens the target of `runs/latest.jsonl`.
Events for that run are read from the opened file.
If no run has been recorded, it prints a short plain message saying so.

The interactive display is an inline terminal UI rather than a full-screen application.
It does not enter the alternate screen.

The **ANSI visual profile** uses the user's standard terminal palette rather than fixed RGB values.
The following **golden frames** define its exact information order, terminal-cell geometry, and spacing at their stated widths.
All shown spaces are significant, and unused cells contain default-style spaces through the stated width.

### Running at 64 columns

```text
18 / 40 ━━━━━━━━━━━━━━━━━━━━━━╺━━━━━━━━━━━━━━━━━━━━━━━━━━ 1h 4m
✓ V       43s
× K9m  1m 11s
▻ KD   2m 27s                                  18 changed files
  Can you find a critical high-confidence bug with a concrete …
  expected: no
───────────────────────────────────────────────────────────────
› g2  L  nO  r8  UH  0Y  kK  kg  d  8  Yg  Sh  3n  4W  2g  u  t
```

### Failure at 88 columns

```text
19 / 40 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━╺━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 4m 12s
✓ V       43s
✓ K9m  1m 11s
× KD   2m 27s
  Can you find a critical high-confidence bug with a concrete failing scenario?
  expected: no
  observed: yes
  evidence: A failed evaluator result is omitted from the completed count, so the final
  ┆ result can appear successful.
───────────────────────────────────────────────────────────────────────────────────────
› g2  L  nO  r8  UH  0Y  kK  kg  d  8  Yg  Sh  3n  4W  2g  u  t  3a  NR  l  UZ  🏁
```

### Success at 88 columns

```text
40 / 40 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 4m 18s
✓ All checks passed.
```

Running and failed frames reserve no empty detail rows.
A successful frame contains only its two shown rows.

The **line-break marker** `↵` replaces each line break in displayed data, and the **truncation marker** `…` marks truncated text; identical source characters retain their surrounding data style.

The golden-frame spans use these styles:

| Span | ANSI foreground | ANSI attribute |
| --- | --- | --- |
| ordinary text and values | terminal default | normal |
| count | terminal default | bold |
| duration | gray | normal |
| labels | gray | bold |
| ticker items | gray | dim |
| line-break and truncation markers | blue | normal |
| unfilled progress, separators, and `›` | dark gray | bold |
| active progress | cyan | normal |
| active `▻` | bright yellow | normal |
| active current short ID | bright yellow | bold |
| recent successful `✓` and short ID | bright green | normal |
| recent failed `×` and short ID | bright red | normal |
| pending after previous pass | bright blue | normal |
| pending after previous failure | bright red | normal |
| pending without previous result | gray | normal |
| failed progress, failed current `×`, error, and observed mismatch | bright red | normal |
| failed current short ID | bright red | bold |
| successful progress | bright green | normal |
| successful final `✓` | bright green | bold |

Active styles remain until a check-finish event even if an earlier evaluation failed, after which failed styles apply when any evaluation-finish event records `fail`.

## Adaptive rendering

`terminal_width` is the current usable width of stdout, has no fixed maximum, and is combined with displayed grapheme widths for every layout.
Every row uses **display width**, which is `terminal_width - 1` at widths of at least 64 columns and `terminal_width` otherwise; the reserved final cell remains a default-style space.

Duration differences are clamped at zero and rendered by `humanize::compact_duration`.
Run duration uses `now` before a check-finish event and that event's timestamp afterward.
Evaluation duration uses its evaluation-start and evaluation-finish timestamps, or `now` while active.

The progress row follows this calculation, where `paint` applies a semantic style from the table above:

```text
count_text = completed + " / " + total
time_text = duration(run_endpoint - run_started)
width = display_width - cells(count_text) - cells(time_text) - 2
halves = 2 * width if total == 0 else floor(2 * width * completed / total)
full = floor(halves / 2)
fill = successful progress if successful
       else failed progress if failed
       else active progress

if halves == 2 * width: bar = paint(fill, "━" * width)
else if halves is odd:  bar = paint(fill, "━" * full + "╸") + paint(unfilled progress, "━" * (width - full - 1))
else if full > 0:       bar = paint(fill, "━" * full) + paint(unfilled progress, "╺" + "━" * (width - full - 1))
else:                   bar = paint(unfilled progress, "━" * width)

progress = paint(count, count_text) + " " + bar + " " + paint(duration, time_text)
```

Before layout, control characters other than line breaks become one space.
Question, expected, observed, and generated status text retain the longest complete grapheme prefix that leaves one cell for the truncation marker when their row would overflow the display width.
A one-cell truncation result is the truncation marker, and a zero-cell result is empty.
Evidence is never truncated.
Its first row follows `  evidence: `; continuation rows begin with `  ┆ ` and use the remaining width.
It wraps at word boundaries, splitting an overlong word only at a grapheme boundary.

Up to two of the most recent evaluations completed before the displayed evaluation appear above it.
They retain evaluation order and use the corresponding table style for their status.

Those rows and the current row share a short-ID field whose width is the widest displayed short ID.
Short IDs are left-aligned in that field.
Their duration field is as wide as the longest displayed duration and is right-aligned.
Each row consists of its marker, one space, the short-ID field, two spaces, and the duration field.
The current row keeps the complete short ID when it fits within the display width.
Its marker is `▻` while no result exists, then `✓` for `pass` or `×` for `fail`.

An unfinished current evaluation shows its latest `ticker`.
Ticker items are joined by `  ·  `.
If the joined ticker fits after the duration with at least two spaces between them, it is right-aligned within the display width.
Otherwise, those two spaces are followed by a viewport using the remaining cells.
The viewport continuously moves the cyclic sequence of ticker items and separators from right to left; the last and first items have the same separator between them.
Complete grapheme clusters are preserved.
When its ticker is updated, the viewport finishes the current item and resumes at the first item whose value did not occur in the previous ticker.

The question and expected answer follow the current row, and a failed evaluation additionally shows the error or observed answer and evidence fields that exist.
A muted `─` separator spans the display width.

The **pending row** follows the separator, begins with `› `, and joins remaining short IDs with exactly two spaces.
It greedily retains complete leading IDs that fit within the display width and never uses an ellipsis.
It appends `  🏁` only when every remaining ID and the complete marker fit, treating `🏁` as two cells.

Between evaluations, the most recently completed evaluation remains the displayed evaluation; if none has completed, the current row is `▻ Waiting for evaluation`.
Pending then includes every unfinished short ID.

Implementations may substitute a fallback for the ANSI visual profile.
Exact cell-buffer snapshots target the ANSI visual profile.

`canon status` changes terminal state only when stdout is interactive and hides the cursor while watching.
Before returning, it leaves the cursor immediately after the displayed frame, restores its prior visibility, and restores every terminal mode it changed.

## Snapshot and watch behavior

`canon status` first renders the current state of the opened status log.
Without `--watch`, it then exits.

With `--watch`, `canon status` continuously follows `runs/latest.jsonl` across successive runs.
If successive runs are discovered by polling `runs/latest.jsonl`, at least 10 seconds elapse between polling attempts.
Appended events update the display.
When cursor control is available, the **redraw origin** is the active cursor position at the first cell of the displayed inline frame.
`canon status` establishes the redraw origin before writing the first watch frame and returns the hidden cursor to it after every frame write that precedes another wait.
Each redraw begins at the redraw origin, erases from there through the end of the display, and writes the complete current frame without accumulating earlier frames in terminal scrollback.
Run and evaluation durations advance locally without new events.
Terminal resize affects the next frame, and triggers an immediate responsive redraw when cursor control is available.

A completed run's final state remains displayed until a later run is available.
