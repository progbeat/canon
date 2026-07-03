use super::expansion::merge_raw_expectation_common_defaults;
use crate::config_types::{RawExpectationFields, RawExpectationItem, RawExpectationItemForm};

pub(super) fn merge_include_generator_fields_as_item_fields(
    items: &mut [RawExpectationItem],
    generator: &RawExpectationFields,
) {
    // An include item is an Expectations generator item. Presets says fields on
    // a generator item count as fields on each generated expectation item, so
    // this merge happens before item preset/default resolution.
    for item in items {
        let RawExpectationItem::Unresolved(fields) = item else {
            unreachable!("included items are raw until include generator fields are merged");
        };
        merge_generator_fields(fields, generator);
    }
}

fn merge_generator_fields(fields: &mut RawExpectationFields, generator: &RawExpectationFields) {
    let declared_form = fields.declared_item_form();
    // Generator fields count as generated item fields before preset/default
    // resolution, but the merge stays form-aware: explicit items inherit
    // explicit-form fields, path generators inherit path-generator fields, and
    // unresolved/include items can still inherit either form's fields.
    if fields.a.is_none() {
        fields.a = generator.a.clone();
    }
    match declared_form {
        Some(RawExpectationItemForm::Explicit) => {
            if fields.explicit_q.is_none() {
                fields.explicit_q = generator.explicit_q.clone();
            }
        }
        Some(RawExpectationItemForm::Generator) => {
            merge_generator_path_fields(fields, generator);
        }
        Some(RawExpectationItemForm::Include) | None => {
            if fields.explicit_q.is_none() {
                fields.explicit_q = generator.explicit_q.clone();
            }
            merge_generator_path_fields(fields, generator);
        }
    }
    if fields.common.settings.preset.is_none() {
        fields.common.settings.preset = generator.common.settings.preset.clone();
    }
    merge_raw_expectation_common_defaults(&mut fields.common, &generator.common);
}

fn merge_generator_path_fields(
    fields: &mut RawExpectationFields,
    generator: &RawExpectationFields,
) {
    if fields.q_template.is_none() {
        fields.q_template = generator.q_template.clone();
    }
    if fields.path.is_none() {
        fields.path = generator.path.clone();
    }
}
