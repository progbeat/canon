# Presets

A check config contains a `presets` mapping.

An expectation item's `preset` field selects a preset by name. If `preset` is absent, the selected preset is `default`.

**preset lookup** decides a setting value for an expectation: expectation item first, selected resolved preset second, implementation default last.

For generated expectations, fields on the generator item count as fields on the generated expectation item.
