use super::*;

#[test]
fn log_timestamp_parser_rejects_invalid_calendar_dates() {
    assert!(parse_record_timestamp("2025-04-31T00:00:00Z").is_none());
    assert!(parse_record_timestamp("2025-02-29T00:00:00Z").is_none());
    assert!(parse_record_timestamp("2024-02-29T00:00:00Z").is_some());
}
