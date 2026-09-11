# tui / components

`tui` owns reusable terminal interaction, components, and styling through `ratatui`; callers supply data, actions, and global TUI stylesheet classes.
Geometry uses terminal-cell widths and preserves complete grapheme clusters.
Cell-range overlays style each intersecting grapheme as a whole.
Rows fill their allocated width, including unused cells; overlays reserve no layout space.
XML-like fragments describe component parts and slots; `{...}` binds local data or state, `class` assigns stylesheet classes, and `overlay` patches already styled cells.

## Terminal session

A **fullscreen session** renders its root in the terminal's available area, requiring an interactive terminal, using the alternate screen and mouse input, parking the cursor bottom-right, and restoring terminal state on every exit path.
Input feedback is rendered independently of background data updates, including pressed-state changes on mouse-down and mouse-up.

## Frame layout

A **frame layout** arranges caller-supplied slots vertically within its allocated area:

```xml
<column class="screen">
  <slot name="header" height="1" />
  <slot name="content" height="remaining" />
  <slot name="footer" height="1" />
</column>
```

The caller supplies a minimum size and fallback message; below that size, the message uses `error`, is the only content, and is clipped to the frame width.

## Progress row

A **progress row** receives count text, completed and total counts, duration text, and caller-supplied fill and optional duration classes:

```xml
<row gap="1">
  <text class="strong">{count}</text>
  <bar filledClass="{fillClasses}" unfilledClass="track">{bar}</bar>
  <text class="muted {durationClasses}">{duration}</text>
</row>
```

Duration is ellipsized to `max(0, W - cells(count) - 2)` cells before calculating the bar.
For row width `W`, it renders:

```text
bar_width = max(0, W - cells(count) - cells(duration) - 2)
halves = 2 * bar_width                                  if total == 0
         floor(2 * bar_width * completed / total)       otherwise
full = floor(halves / 2)

bar = fill("━" * bar_width)                                              if full == bar_width
      fill("━" * full + "╸") + unfilled("━" * (bar_width - full - 1))  if halves is odd
      fill("━" * full) + unfilled("╺" + "━" * (bar_width - full - 1))  if full > 0
      unfilled("━" * bar_width)                                          otherwise
```

The result is clipped to the row width.

## List viewport

A **list viewport** renders an ordered sequence of primary rows, each optionally followed by child rows:

```xml
<primary-row overlay="{selected ? 'selection' : ''}">
  <number-gutter when="{numbering}" class="index {selected ? 'current' : ''}" />
  <slot name="content" />
</primary-row>
```

The optional **number gutter** contains the primary row's one-based absolute sequence number, right-aligned, followed by two separating cells.
It is blank on child rows.
Its digit width and application-defined column widths are calculated from primary rows visible in the resulting viewport; when none is visible, the selected primary row supplies the sizing data.
Use the smallest digit width that fits those numbers without making unchanged data and viewport geometry alternate between layouts.

The caller may restrict the sequence to a suffix. For viewport height `V`, content height `N`, and a primary row at `P` with block height `B` including children, positions are zero-based from that suffix and exclude virtual rows:

```text
top_padding(B) = floor((V - min(V, B)) / 2)
max_scroll = max(0, N - V, last_primary_position - top_padding(last_block_height))
scroll_extent = V + max_scroll
center(P, B) = clamp(P - top_padding(B), 0, max_scroll)
```

Virtual rows below content contain `~` with class `index` in their first cell and no number.
An empty sequence has no selection or scrolling.

A reveal request changes scroll only enough to bring the primary row into view, placing an offscreen row at the nearest viewport edge.
Collapsing children preserves the center `c` of the selected block's previously visible rows, measured within the viewport, by setting scroll to `clamp(min(P, ceil(P + (B - 1) / 2 - c)), 0, max_scroll)` for the new block; when none of it was visible, reveal its primary row.
Continuous centering takes precedence over collapse preservation and reveal requests. Other changes preserve scroll, clamped to its bounds.

## Inline field blocks

An **inline field block** lays out these parts left to right:

```xml
<row class="field">
  <strip class="fold" width="1">{expandable ? (expanded ? '-' : '+') : ' '}</strip>
  <strip class="edge" width="1">▏</strip>
  <content labelClass="label">{contentRows}</content>
</row>
```

Both strips repeat their character for every content row, with no gaps between parts.
Field parity counts displayed blocks from one; all rows in a block share it.

Clicking any row of an expandable field invokes its toggle action.

## Inline value

An **inline value** renders each line break as `↵` and other control characters as spaces.
An ellipsized value retains the longest prefix that leaves one cell for `…`; a clipped value uses no truncation marker. A zero-cell value is empty.
Generated markers use `escape`, separately from identical literal source characters; a caller may instead style the entire value uniformly.

## Wrapped text

**Wrapped text** preserves source lines, blank lines, and leading spaces; tabs expand to four-cell stops.
Soft wraps prefer whitespace boundaries, discard separating whitespace, and split overlong words only at grapheme boundaries.
Each continuation repeats the source line's indentation when it leaves room for content.
An optional label appears only on the first row; subsequent rows receive the caller's additional `continuation_indent` cells.
Wrap width is `max(1, content_width - max(cells(label), continuation_indent))` for all rows.

## Path forest

A **path forest** receives ordered slash-delimited paths and their directory classification.
Roots and siblings preserve first-occurrence order; roots have no branch prefix.
Descendants use `├─ `, `└─ `, and `│  ` with class `branch` and three cells per level, placing child branches under their parent's first character.
Each unbranched chain becomes one slash-delimited label; directory labels end in `/`.
Rows use the caller's indentation and clip at the available width.

## Scrollbar

A **scrollbar** overlays a trackless background handle on the last two viewport columns when scroll extent exceeds viewport height.
The handle uses `scroll-thumb`, applied after selection and pressed overlays.
For scroll extent `T`, viewport height `V`, and scroll offset `S`, it covers `[start, end)`:

```text
start = floor(S * V / T)
end   = min(V, max(start + 1, ceil((S + V) * V / T)))
```

`S` is relative to the scrollable content's start.
Clicking either of the last two viewport columns places the handle's center as close as possible to the clicked row within the scroll bounds, taking precedence over underlying row actions.

## Overview ruler

An **overview ruler** uses viewport height `V`, scroll extent `T`, and content row positions `P` relative to the scrollable content's start.
It places each marker at row `min(V - 1, floor(P * V / T))`.
Markers sharing a ruler row retain the last five in the caller's order and occupy adjacent cells, right-aligned to the viewport edge.
Each uses its caller-supplied overlay classes, applied after all other layers.
The ruler is absent whenever the scrollbar is absent.

## Click gestures

A mouse gesture that moves to another cell or reports a keyboard modifier cancels click actions, including when released over the original target.

## Hint

A **hint** consists of keys and a label:

```xml
<row gap="1" overlay="{pressed ? 'pressed' : ''}">
  <keys class="key" separator="/" separatorClass="separator">{keys}</keys>
  <text class="action">{label}</text>
</row>
```

Hints may be clicked.
An actionable hint is pressed from mouse-down until mouse-up and invokes its action only when released over the same hint.
Its hit area covers its key, label, and internal separators, excluding spacing between hints.

## Hint bar

A **hint bar** arranges caller-supplied groups:

```xml
<row class="surface">
  <groups align="left">{contextualGroups}</groups>
  <groups align="right">{persistentGroups}</groups>
</row>
```

Diagnostic text uses `muted dim`.
Hints within a group are separated by two spaces; groups are separated by `  │  ` with class `separator`.
The right groups end with one padding cell; at least two cells separate the left and right portions.
When space is insufficient, keep the longest complete prefix of left hints that fits with `  │  …` appended, or just `…` if no hint fits, while the right groups remain intact.

## Keys page

A **keys page** displays a caller-supplied shortcut map as sections with keys and sentence-case descriptions in aligned columns within its allocated area:

```xml
<section>
  <heading class="heading">{title}</heading>
  <gap height="1" />
  <row repeat="{shortcuts}">
    <keys class="key" maxWidth="17" separator=" / " separatorClass="separator">{keys}</keys>
    <text class="action" x="18">{description}</text>
  </row>
</section>
```

One blank row separates sections.
A column's natural width is `max(widest_heading, 18 + widest_description)`.
For allocated width `W`, split the ordered sections into two consecutive groups when both natural widths plus four separating cells fit within `W - 2`.
Choose the split with the smallest maximum height, then height difference, then total width; ties use the earliest split.
Otherwise use one column capped at `W - 2`; overflowing cell content is clipped.
The block is horizontally centered with at least one cell of left padding, rounding its left offset down.
Caller-supplied `paddingTop`, defaulting to zero, reserves fixed blank rows above the scrolling content within that area.
The page scrolls vertically when needed and uses the shared scrollbar.
