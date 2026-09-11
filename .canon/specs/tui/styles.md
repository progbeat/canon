# tui / styles

The **global TUI stylesheet** resolves colors and modifiers for component parts, states, and caller-assigned classes independently of command logic.

The CSS-like contract below uses xterm-256 color indices; `default` means the terminal default.
Omitted properties inherit their containing style. Matching declarations combine property by property, using CSS specificity and then source order; class-list order has no effect.
`bold` and `dim` are independent Boolean modifiers.

```css
.screen { foreground: default; background: default; bold: false; dim: false; }

.muted         { foreground: 252; }
.subtle        { foreground: 60; dim: true; }
.success       { foreground: 42; }
.success-muted { foreground: 35; dim: true; }
.error         { foreground: 203; }
.warning       { foreground: 221; }
.info          { foreground: 75; }
.busy          { foreground: 44; }
.escape        { foreground: 69; }
.track         { foreground: 243; bold: true; }
.strong        { bold: true; }
.dim           { dim: true; }

.label, .branch, .heading { foreground: 172; }
.label, .heading         { bold: true; }
.branch                  { dim: true; }

.index         { foreground: 58; dim: true; }
.index.current { dim: false; }

.field > .fold { foreground: 136; bold: true; dim: true; }
.field > .edge { background: default; dim: true; }
.field:nth-child(odd) > .fold  { background: 234; }
.field:nth-child(even) > .fold { background: 236; }
.field:nth-child(odd) > .edge  { foreground: 239; }
.field:nth-child(even) > .edge { foreground: 243; }

.surface   { foreground: 252; background: 234; }
.key       { foreground: 255; bold: true; }
.action    { foreground: 245; }
.separator { foreground: 239; }

.selection    { background: 235; }
.pressed      { background: 235; }
.scroll-thumb { background: 236; }
.mark-error   { background: 52; }
.mark-warning { background: 58; }
```

Overlays patch only their declared properties onto the already styled cells, preserving glyphs and every other property.
