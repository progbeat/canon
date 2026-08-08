/// A canonical answer in the shared evaluation-response string domain.
#[derive(Debug)]
pub(crate) struct EvaluationAnswer(String);

impl EvaluationAnswer {
    pub(crate) fn new(answer: String) -> EvaluationAnswer {
        EvaluationAnswer(answer)
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}
