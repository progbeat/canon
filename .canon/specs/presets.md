# Presets

A check config contains a `presets` mapping.

Each preset is a named set of expectation item field defaults.

An expectation item's `preset` field selects one or more presets by name, joined by `+`. If `preset` is absent, the selected preset is `default`.

For every field of an expectation item, the value is resolved in this order: expectation item, selected presets from right to left, then implementation default.
