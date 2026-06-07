use crate::check::command::output::CheckProgressOutput;

pub(super) fn cancel_progress_on_error<T>(
    result: Result<T, String>,
    progress: &mut Option<CheckProgressOutput>,
) -> Result<T, String> {
    match result {
        Ok(value) => Ok(value),
        Err(err) => {
            if let Some(progress) = progress.take() {
                progress
                    .cancel()
                    .map_err(|progress_err| format!("{}; {}", err, progress_err))?;
            }
            Err(err)
        }
    }
}
