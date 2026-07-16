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
    pub(super) answer: String,
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
    let mut transcript = format!("$ {question}\n");
    transcript.push_str(&String::from_utf8_lossy(&transcript_output));
    Ok(ShellEvaluation {
        answer: code.to_string(),
        transcript,
    })
}

#[cfg(test)]
mod tests {
    use super::evaluate;
    use std::path::Path;

    #[cfg(unix)]
    const INTERLEAVED_COMMAND: &str =
        "printf 'stdout\\n'; printf 'stderr\\n' >&2; printf 'after\\n'; exit 3";
    #[cfg(windows)]
    const INTERLEAVED_COMMAND: &str = "echo stdout& 1>&2 echo stderr& echo after& exit /b 3";

    #[test] // xpec: 1r,nF
    fn shell_evaluation_preserves_stdout_stderr_order_in_one_transcript() {
        let evaluation = evaluate(Path::new("."), INTERLEAVED_COMMAND).unwrap();

        assert_eq!(evaluation.answer, "3");
        let stdout = evaluation.transcript.find("stdout").unwrap();
        let stderr = evaluation.transcript.find("stderr").unwrap();
        let after = evaluation.transcript.find("after").unwrap();
        assert!(
            stdout < stderr && stderr < after,
            "{}",
            evaluation.transcript
        );
    }
}
