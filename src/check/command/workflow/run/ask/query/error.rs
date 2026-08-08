#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::check::command::workflow::run::ask) enum AskQueryError {
    Unreported(String),
    Reported(String),
}

impl std::fmt::Display for AskQueryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AskQueryError::Unreported(message) | AskQueryError::Reported(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl From<String> for AskQueryError {
    fn from(message: String) -> AskQueryError {
        AskQueryError::Unreported(message)
    }
}
