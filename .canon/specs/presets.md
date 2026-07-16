# Presets

A check config contains a `presets` mapping.

Each preset is a named set of expectation item field defaults.

An expectation item's `preset` field selects a preset by name. If `preset` is absent, the selected preset is `default`.

For every field of an expectation item, the value is resolved in this order: expectation item first, selected resolved preset second, implementation default last.
