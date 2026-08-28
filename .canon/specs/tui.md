# TUI

All functionality specific to terminal user interfaces belongs to the `tui` namespace.

## Terminal operations

`tui` provides terminal operations including those below.
`tui::is_terminal()` reports whether the application is attached to a terminal.
`tui` automatically restores every terminal-state change on every exit where cleanup can run.
`tui::hide_cursor()` hides the cursor, prevents terminal input from being echoed or controlling its position, and gives `tui` exclusive control of it.
`tui::show_cursor()` makes the cursor visible and gives the user control of its position.

## Frame diff algorithm

A **frame** is the complete desired styled-cell content at one terminal geometry.
Its width is the terminal width and its height is arbitrary.
The algorithm compares two frames of the same width using their styled cells and the encoded byte lengths of available terminal operations, without executing those operations.
The available operations and their encodings are implementation-defined.

`tui` performs this comparison in linear time over the combined number of frame cells:

1. Identical styled rows are matched in order as anchors; rows between anchors are paired in order, with surplus rows inserted or removed.
2. Each differing row pair uses dynamic programming to find a minimum-byte alignment over previous and next cell positions. Both positions move only forward, their difference never exceeds one, and each transition advances either position or both.

A **complete redraw** removes the previous frame and renders the next frame from scratch.
The algorithm returns whichever has fewer encoded bytes: the resulting update or a complete redraw; equal frames produce an empty update.

## Frame lifecycle

A successfully presented frame becomes the previous frame.
Terminal resize invalidates it so that the next update is a complete redraw.

## Inline mode

**Inline mode** replaces successive frames in place without accumulating them in terminal scrollback.
When the cursor is hidden by `tui::hide_cursor()`, the first frame starts at the first column of the current cursor row.
For such output, every nonfinal frame update immediately returns to that position before any further event is handled, by moving the cursor upward by the number of frame rows it traversed and then issuing a carriage return.
Its final update leaves the cursor at the first column immediately after the final frame.
