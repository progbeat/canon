use crate::config_types::{RawExpectationFields, ResolvedPresetConfig};

fn merge_all_raw_expectation_field_defaults(
    fields: &mut RawExpectationFields,
    defaults: &RawExpectationFields,
) {
    merge_raw_expectation_value_defaults(fields, defaults);
    merge_raw_expectation_explicit_q_default(fields, defaults);
}

fn merge_raw_expectation_value_defaults(
    fields: &mut RawExpectationFields,
    defaults: &RawExpectationFields,
) {
    if fields.a.is_none() {
        fields.a = defaults.a.clone();
    }
    fields.common.fill_missing_from(&defaults.common);
}

fn merge_raw_expectation_explicit_q_default(
    fields: &mut RawExpectationFields,
    defaults: &RawExpectationFields,
) {
    if fields.explicit_q.is_none() {
        fields.explicit_q = defaults.explicit_q.clone();
    }
}

pub(super) fn apply_raw_expansion_item_preset_defaults(
    fields: &mut RawExpectationFields,
    preset: &ResolvedPresetConfig,
) {
    let defaults = raw_expectation_fields_from_preset(preset);
    merge_all_raw_expectation_field_defaults(fields, &defaults);
}

fn raw_expectation_fields_from_preset(preset: &ResolvedPresetConfig) -> RawExpectationFields {
    RawExpectationFields {
        explicit_q: preset.q.clone(),
        a: preset.a.clone(),
        common: preset.common.clone(),
    }
}
