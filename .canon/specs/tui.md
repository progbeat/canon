# TUI

The sole responsibility of the `tui` namespace is providing general-purpose terminal user interface functionality.
It must not own or implement any behavior whose responsibility belongs to the application rather than the terminal interface itself.

## Terminal operations

`tui::run()` applies a requested TUI mode while executing caller-provided work.
The caller decides when and how often frames are presented.
It fails before that work begins when the mode's required terminal operations are unavailable.
`tui` automatically restores every terminal-state change on every exit where cleanup can run.

## Frame diff algorithm

**Frame width** is the finite width provided by `tui` for each frame.
A **frame** is the complete desired styled-cell content at its frame width; its height is arbitrary.
`tui` can present a frame without entering a TUI mode or requiring an attached terminal.
When styled terminal output is unavailable, it emits the frame's text without terminal control sequences.
The algorithm compares two frames of the same frame width using their styled cells and the encoded byte lengths of available terminal operations, without executing those operations.

`tui` performs this comparison in linear time over the combined number of frame cells:

1. Identical styled rows are matched in order as anchors; rows between anchors are paired in order, with surplus rows inserted or removed.
2. Each differing row pair uses dynamic programming to find a minimum-byte alignment over previous and next cell positions. Both positions move only forward, their difference never exceeds one, and each transition advances either position or both.

A **complete redraw** removes the previous frame and renders the next frame from scratch.
The algorithm returns whichever has fewer encoded bytes: the resulting update or a complete redraw; equal frames produce an empty update.

## Frame lifecycle

A successfully presented frame becomes the previous frame.
A frame-width change invalidates it so that the next update is a complete redraw.

## Inline mode

**Inline mode** requires terminal cursor control and replaces successive frames in place without accumulating them in terminal scrollback.
While it is active, `tui::run()` hides the cursor, prevents terminal input from being echoed or controlling its position, and gives `tui` exclusive control of it.
The first frame starts at the first column of the current cursor row.
Every nonfinal frame update immediately returns to that position before any further event is handled, by moving the cursor upward by the number of frame rows it traversed and then issuing a carriage return.
The final update leaves the cursor at the first column immediately after the final frame.
