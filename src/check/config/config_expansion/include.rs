use crate::config_types::{
    RawExpectationCommonConfig, RawExpectationFields, RawExpectationItem, RawExpectationItemForm,
};

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
    merge_generator_common_config(&mut fields.common, &generator.common);
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

fn merge_generator_common_config(
    config: &mut RawExpectationCommonConfig,
    generator: &RawExpectationCommonConfig,
) {
    if config.settings.preset.is_none() {
        config.settings.preset = generator.settings.preset.clone();
    }
    if config.settings.models.is_none() {
        config.settings.models = generator.settings.models.clone();
    }
    if config.settings.thinking.is_none() {
        config.settings.thinking = generator.settings.thinking.clone();
    }
    if config.settings.ignore.is_none() {
        config.settings.ignore = generator.settings.ignore.clone();
    }
    if config.settings.plugins.is_none() {
        config.settings.plugins = generator.settings.plugins.clone();
    }
    if config.cooldown.is_none() {
        config.cooldown = generator.cooldown.clone();
    }
    merge_generator_text(&mut config.question_context, &generator.question_context);
    merge_generator_text(&mut config.diff_from, &generator.diff_from);
    merge_generator_text(&mut config.target, &generator.target);
}

fn merge_generator_text(value: &mut Option<String>, generator: &Option<String>) {
    if value.is_none() {
        *value = generator.clone();
    }
}
