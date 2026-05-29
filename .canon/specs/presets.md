# Presets

A check config contains a `presets` mapping.

An expectation item's `preset` field selects a preset by name. If `preset` is absent, the selected preset is `default`.

A preset may include `extends` to name another preset. After parsing presets, `canon` resolves `extends` so each preset contains its inherited setting keys directly. A child preset overrides only the setting keys it defines. Preset inheritance cycles are invalid.

**preset lookup** decides a setting value for an expectation: expectation item first, selected resolved preset second, implementation default last.

For generated expectations, fields on the generator item count as fields on the generated expectation item.
