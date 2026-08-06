use crate::check::core::EvaluationAnswer;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::thread;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

pub(super) struct ShellEvaluation {
    pub(super) answer: EvaluationAnswer,
    pub(super) transcript: String,
}

pub(super) fn evaluate(root: &Path, question: &str) -> Result<ShellEvaluation, String> {
    let (mut transcript_reader, transcript_writer) = std::io::pipe()
        .map_err(|error| format!("failed to create shell transcript pipe: {error}"))?;
    let stderr_writer = transcript_writer
        .try_clone()
        .map_err(|error| format!("failed to share shell transcript pipe: {error}"))?;
    let transcript_thread = thread::spawn(move || {
        let mut transcript = Vec::new();
        transcript_reader
            .read_to_end(&mut transcript)
            .map(|_| transcript)
    });

    let mut command = imp::command(question);
    let output = command
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(transcript_writer))
        .stderr(Stdio::from(stderr_writer))
        .output();
    drop(command);

    let command_output = output.map_err(|error| format!("failed to run shell xpec: {error}"));
    let transcript_output = transcript_thread
        .join()
        .map_err(|_| "failed to collect shell transcript: reader thread panicked".to_string())?
        .map_err(|error| format!("failed to collect shell transcript: {error}"));
    let output = command_output?;
    let transcript_output = transcript_output?;
    let code = output
        .status
        .code()
        .ok_or_else(|| "shell xpec terminated without an exit code".to_string())?;
    let transcript = shell_transcript(question, transcript_output)?;
    Ok(ShellEvaluation {
        // [MH,Eg] Shell owns and normalizes its integer process-status source
        // before constructing the shared evaluation-response answer.
        answer: EvaluationAnswer::new(code.to_string()),
        transcript,
    })
}

fn shell_transcript(question: &str, output: Vec<u8>) -> Result<String, String> {
    let output = String::from_utf8(output)
        .map_err(|err| format!("shell xpec transcript must be valid UTF-8: {err}"))?;
    Ok(format!("$ {question}\n{output}"))
}

#[cfg(test)]
mod tests {
    use super::{evaluate, shell_transcript};
    use std::path::Path;

    #[cfg(unix)]
    const INTERLEAVED_COMMAND: &str =
        "printf 'stdout\\n'; printf 'stderr\\n' >&2; printf 'after\\n'; exit 3";
    #[cfg(windows)]
    const INTERLEAVED_COMMAND: &str = "echo stdout& 1>&2 echo stderr& echo after& exit /b 3";

    #[test] // xpec: 1r,MH,Eg
    fn shell_evaluation_stringifies_exit_code_and_preserves_transcript_order() {
        let evaluation = evaluate(Path::new("."), INTERLEAVED_COMMAND).unwrap();

        assert_eq!(evaluation.answer.into_string(), "3");
        let stdout = evaluation.transcript.find("stdout").unwrap();
        let stderr = evaluation.transcript.find("stderr").unwrap();
        let after = evaluation.transcript.find("after").unwrap();
        assert!(
            stdout < stderr && stderr < after,
            "{}",
            evaluation.transcript
        );
    }

    #[test] // xpec: gO,Eg
    fn shell_evaluation_reports_non_utf8_transcript() {
        assert!(shell_transcript("command", vec![0xff]).is_err());
    }
}
