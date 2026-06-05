pub(crate) fn compact_json_string_array(values: &[String]) -> String {
    serde_json::to_string(values).expect("serializing a JSON string array cannot fail")
}
