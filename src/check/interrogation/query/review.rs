use crate::check::core::{
    QueryResult, ERROR_INSUFFICIENT_EVIDENCE, ERROR_INVALID_QUESTION, ERROR_UNPARSABLE,
};

pub(super) fn human_review_reason(result: &QueryResult) -> Option<&'static str> {
    match result.answer.error.as_deref() {
        Some(ERROR_INSUFFICIENT_EVIDENCE) => Some("insufficient evidence"),
        Some(ERROR_INVALID_QUESTION) => Some("invalid question"),
        Some(ERROR_UNPARSABLE) => Some("unparsable evaluator response"),
        None => None,
        Some(_) => Some("unknown evaluator error"),
    }
}

#[cfg(test)]
mod tests {
    use super::human_review_reason;
    use crate::check::core::{ParsedAnswer, QueryResult};

    #[test]
    fn unknown_query_error_requires_human_review() {
        let result = QueryResult {
            answer: ParsedAnswer::error("future-error".to_string(), "details".to_string()),
        };

        assert_eq!(
            human_review_reason(&result),
            Some("unknown evaluator error")
        );
    }
}
