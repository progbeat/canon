use super::*;

#[test]
fn expectation_id_is_twenty_base62_characters() {
    let id = expectation_id("Question?");

    assert_eq!(id.len(), 20);
    assert!(id.chars().all(|ch| ch.is_ascii_alphanumeric()));
    assert_eq!(id, expectation_id("Question?"));
    assert_ne!(id, expectation_id("Different question?"));
}
