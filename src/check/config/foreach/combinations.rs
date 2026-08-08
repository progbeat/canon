use super::binding::ForeachBinding;
use minijinja::value::Value;
use std::collections::BTreeMap;

pub(super) fn foreach_combinations(
    bindings: Vec<ForeachBinding>,
) -> Result<Vec<BTreeMap<String, Value>>, String> {
    let mut combinations = vec![BTreeMap::new()];
    for binding in bindings {
        let capacity = combinations
            .len()
            .checked_mul(binding.choices.len())
            .ok_or_else(|| "!foreach combination count exceeds platform limits".to_string())?;
        let mut expanded = Vec::with_capacity(capacity);
        for combination in combinations {
            for choice in &binding.choices {
                let mut copy = combination.clone();
                copy.insert(binding.name.clone(), choice.clone());
                // xpec: jM
                // Each occurrence in the selected Cartesian product is one
                // combination and contributes one copy, even when its binding
                // values equal those of another selected occurrence.
                expanded.push(copy);
            }
        }
        combinations = expanded;
    }
    Ok(combinations)
}
