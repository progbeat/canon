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

Only `canon check` writes status logs, creating one append-only **status log** per invocation using this layout:

```text
runs/
  <timestamp>-<unique>.jsonl
  latest.jsonl -> <timestamp>-<unique>.jsonl
```

Each line is one complete structured JSON event that is flushed when appended.
A reader ignores an incomplete final line.

The writer adds a timestamp to every event's common envelope.
Status logs contain only the events below.

The **initial event** contains every **Selected** expectation's full ID in evaluation order.
The expectations displayed by `canon status` are exactly this ordered set.
Each entry also contains the status from its pre-run `last.json`, or `null` when absent, reusing the state already loaded by `canon check`.
The initial event also records what `canon status` needs to obtain the same **Collected** expectations without copying expectation definitions into the status log.
After flushing this event, `canon check` atomically replaces the symbolic link `runs/latest.jsonl` with one pointing to the new file.

An **evaluation-start event** identifies the expectation whose evaluation began by full ID.

An **evaluation-finish event** identifies the expectation by full ID, records its current status as `pass` or `fail`, and, for `fail`, includes only the observed answer and evidence fields that exist.

A **check-finish event** marks the normal end of evaluation, after which no further evaluation starts in that run.

The initial ordered list determines the total count.
The number of evaluation-finish events determines completed progress.
Before a check-finish event, `canon status` displays the run as running.
After it, the run succeeds if every initial ID has a `pass` evaluation-finish event and fails otherwise.
Timestamps are not displayed as wall-clock times.

Old status logs are removed automatically according to a bounded retention policy.
Cleanup never removes the file currently being written or the target of `runs/latest.jsonl`.

## Status display

`canon status` resolves `runs/latest.jsonl` once and opens its target.
All subsequent event reads use that opened file until the command exits.
If no run has been recorded, it prints a short plain message saying so.

The interactive display is an inline terminal UI rather than a full-screen application.
It does not enter the alternate screen.

The **ANSI visual profile** uses the user's standard terminal palette rather than fixed RGB values.
The following **golden frames** define its exact information order, terminal-cell geometry, and spacing at their stated widths.
All shown spaces are significant, and unused cells contain default-style spaces through the stated width.

### Running at 64 columns

```text
18 / 40  ━━━━━━━━━━━━━━━━━━━━━╺━━━━━━━━━━━━━━━━━━━━━━━━━  4m 12s
✓ V       43s
× K9m  1m 11s
◆ KD   2m 27s
  Can you find a critical high-confidence bug with a concrete f…
  expected: no
────────────────────────────────────────────────────────────────
› g2  L  nO  r8  UH  0Y  kK  kg  d  8  Yg  Sh  3n  4W  2g  u  t
```

### Failure at 88 columns

```text
19 / 40  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━╸━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  4m 12s
✓ V       43s
✓ K9m  1m 11s
◆ KD   2m 27s
  Can you find a critical high-confidence bug with a concrete failing scenario?
  expected: no
  observed: yes
  evidence: A failed evaluator result is omitted from the completed count, so the final
            result can appear successful.
────────────────────────────────────────────────────────────────────────────────────────
› g2  L  nO  r8  UH  0Y  kK  kg  d  8  Yg  Sh  3n  4W  2g  u  t  3a  NR  l  UZ  🏁
```

### Success at 88 columns

```text
40 / 40  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  4m 18s
✓ All checks passed.
```

Running and failed frames reserve no empty detail rows.
A successful frame contains only its two shown rows.

The golden-frame spans use these styles:

| Span | ANSI foreground | Weight |
| --- | --- | --- |
| ordinary text and values | terminal default | normal |
| count | terminal default | bold |
| duration | gray | normal |
| labels | gray | bold |
| unfilled progress, separator, and `›` | dark gray | normal |
| active progress | cyan | normal |
| active `◆` | bright yellow | normal |
| active current short ID | bright yellow | bold |
| recent successful `✓` and short ID | bright green | normal |
| recent failed `×` and short ID | bright red | normal |
| pending after previous pass | bright blue | normal |
| pending after previous failure | bright red | normal |
| pending without previous result | gray | normal |
| failed progress, `◆`, and observed mismatch | bright red | normal |
| failed current short ID | bright red | bold |
| successful progress | bright green | normal |
| successful final `✓` | bright green | bold |

Active styles remain until a check-finish event even if an earlier evaluation failed, after which failed styles apply when any evaluation-finish event records `fail`.

## Adaptive rendering

`terminal_width` is the current usable width of stdout, has no fixed maximum, and is combined with displayed grapheme widths for every layout.

Durations below one minute use `Ss` with unpadded seconds.
Longer durations use `Xm SSs`, with unpadded minutes and two-digit seconds.
Duration deltas are clamped at zero and floored to whole seconds.
Run duration uses `now` before a check-finish event and that event's timestamp afterward.
Evaluation duration uses its evaluation-start and evaluation-finish timestamps, or `now` while active.

The progress row follows this calculation, where `paint` applies a semantic style from the table above:

```text
count_text = right_align(completed, digits(total)) + " / " + total
time_text = duration(run_endpoint - run_started)
width = terminal_width - cells(count_text) - cells(time_text) - 4
halves = 2 * width if total == 0 else floor(2 * width * completed / total)
full = floor(halves / 2)
fill = successful progress if successful
       else failed progress if failed
       else active progress

if halves == 2 * width: bar = paint(fill, "━" * width)
else if halves is odd:  bar = paint(fill, "━" * full + "╸") + paint(unfilled progress, "━" * (width - full - 1))
else if full > 0:       bar = paint(fill, "━" * full) + paint(unfilled progress, "╺" + "━" * (width - full - 1))
else:                   bar = paint(unfilled progress, "━" * width)

progress = paint(count, count_text) + "  " + bar + "  " + paint(duration, time_text)
```

Before layout, every tab, line break, escape, or other control character in displayed data becomes one space.
Question, expected, observed, and generated status text retain the longest complete grapheme prefix that leaves one cell for `…` when their row would overflow.
A one-cell truncation result is `…`, and a zero-cell result is empty.
Evidence is never truncated.
It wraps at word boundaries within the cells left after `  evidence: `, splitting an overlong word only at a grapheme boundary.
Every continuation row begins with twelve spaces so its text aligns with the first evidence value cell.

Up to two of the most recent evaluations completed before the displayed evaluation appear above it.
They retain evaluation order and use the corresponding table style for their status.

Those rows and the current row share a short-ID field whose width is the widest displayed short ID.
Short IDs are left-aligned in that field.
Their duration field is as wide as the longest displayed duration and is right-aligned.
Each row consists of its marker, one space, the short-ID field, two spaces, and the duration field.
The current row keeps `◆` and the complete short ID.

The question and expected answer follow the current row, and a failed evaluation additionally shows the observed and evidence fields that exist.
A full-width muted `─` separator follows the context.

The **pending row** follows the separator, begins with `› `, and joins remaining short IDs with exactly two spaces.
It greedily retains complete leading IDs that fit and never uses an ellipsis.
It appends `  🏁` only when every remaining ID and the complete marker fit, treating `🏁` as two cells.

Between evaluations, the recent rows still render and the current row is `◆ Waiting for evaluation`.
Pending then includes every unfinished short ID.

Implementations may substitute a fallback for the ANSI visual profile.
Exact cell-buffer snapshots target the ANSI visual profile.

Cursor control is used only when stdout is interactive.

## Snapshot and watch behavior

`canon status` first renders the current state of the opened status log.
Without `--watch`, it then exits.

With `--watch` on a running status log, appended events update the display.
When cursor control is available, it redraws the inline frame in place without accumulating repeated frames in terminal scrollback.
Run and evaluation durations advance locally without new events.
Terminal resize affects the next frame, and triggers an immediate responsive redraw when cursor control is available.

`canon status --watch` exits after rendering the final successful or failed state.
The final rendered state remains in normal terminal output after exit.
