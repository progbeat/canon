use super::directory::PromptTemplateArtifactDir;
use super::file::write_full_template_command_stdout_artifact;
use minijinja::Error;
use std::path::PathBuf;
use std::sync::Mutex;

pub(crate) const TEMPLATE_OUTPUT_HEAD_BYTES: usize = 8 * 1024;

pub(crate) fn truncated_template_command_output(
    stdout: &[u8],
    template_artifact_dir: &PromptTemplateArtifactDir,
    template_artifact_paths: &Mutex<Vec<PathBuf>>,
) -> Result<String, Error> {
    if stdout.len() <= TEMPLATE_OUTPUT_HEAD_BYTES {
        return Ok(String::from_utf8_lossy(stdout).into_owned());
    }
    // Prompt-template truncation is a token-budget mechanism, not a visibility
    // boundary. The Prompt Templates spec deliberately exposes a readable file
    // containing the same command stdout so the evaluator can inspect more of
    // an already-visible transcript when the head is insufficient. Canon never
    // reads this artifact back as check-run state.
    let path = write_full_template_command_stdout_artifact(template_artifact_dir, stdout)?;
    template_artifact_paths.lock().unwrap().push(path.clone());
    let (mut head, head_lines) = template_command_stdout_head(stdout);
    if !head.ends_with('\n') {
        head.push('\n');
    }
    let total_lines = output_line_count(stdout);
    head.push_str(&format!(
        "[truncated: showing first {} of {} lines; full output: {}]\n",
        head_lines,
        total_lines,
        path.display()
    ));
    Ok(head)
}

fn template_command_stdout_head(stdout: &[u8]) -> (String, usize) {
    let mut end = 0usize;
    let mut head_lines = 0usize;
    for line in stdout.split_inclusive(|byte| *byte == b'\n') {
        if end.saturating_add(line.len()) > TEMPLATE_OUTPUT_HEAD_BYTES {
            break;
        }
        end += line.len();
        head_lines += 1;
    }
    if head_lines > 0 {
        return (
            String::from_utf8_lossy(&stdout[..end]).into_owned(),
            head_lines,
        );
    }

    let end = TEMPLATE_OUTPUT_HEAD_BYTES.min(stdout.len());
    let head = String::from_utf8_lossy(&stdout[..end]).into_owned();
    // No complete line fit in the byte budget. The byte head remains useful,
    // but it must not be reported as one fully shown line.
    (head, 0)
}

fn output_line_count(output: &[u8]) -> usize {
    if output.is_empty() {
        return 0;
    }
    let lines = output.iter().filter(|byte| **byte == b'\n').count();
    if output.ends_with(b"\n") {
        lines
    } else {
        lines + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::filesystem::create_private_dir;
    use std::fs;

    #[test] // xpec: 3a
    fn template_command_output_truncates_large_output() {
        let output = (0..6000)
            .map(|index| format!("line {index}\n"))
            .collect::<String>();
        let output_dir = test_output_dir("truncate");
        let artifact_paths = Mutex::new(Vec::new());
        let rendered = truncated_template_command_output(
            output.as_bytes(),
            &PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &artifact_paths,
        )
        .unwrap();

        assert!(rendered.contains("[truncated: showing first "));
        assert!(rendered.contains("; full output: "));
        assert!(!rendered.contains("[begin untrusted command output"));
        assert!(!rendered.contains("[end untrusted command output"));
        assert_eq!(artifact_paths.lock().unwrap().len(), 1);
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test] // xpec: 3a
    fn partial_oversized_first_line_is_not_counted_as_shown() {
        let output = vec![b'x'; TEMPLATE_OUTPUT_HEAD_BYTES + 1];
        let output_dir = test_output_dir("partial-first-line");
        let artifact_paths = Mutex::new(Vec::new());

        let rendered = truncated_template_command_output(
            &output,
            &PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &artifact_paths,
        )
        .unwrap();

        assert!(rendered.contains("[truncated: showing first 0 of 1 lines; full output: "));
        assert_eq!(fs::read(full_output_path(&rendered)).unwrap(), output);
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test] // xpec: 3a
    fn oversized_non_utf8_stdout_keeps_raw_artifact() {
        let mut output = (0..1200)
            .flat_map(|index| format!("line {index}\n").into_bytes())
            .collect::<Vec<_>>();
        output.extend_from_slice(&[0xff, 0xfe, b'\n']);
        let output_dir = test_output_dir("non-utf8-artifact");
        let artifact_paths = Mutex::new(Vec::new());

        let rendered = truncated_template_command_output(
            &output,
            &PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &artifact_paths,
        )
        .unwrap();
        let path = full_output_path(&rendered);

        assert_eq!(fs::read(path).unwrap(), output);
        assert_eq!(
            artifact_paths.lock().unwrap().as_slice(),
            &[PathBuf::from(path)]
        );
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test] // xpec: 3a,gO
    fn oversized_multibyte_first_line_keeps_a_valid_utf8_head() {
        let output = "é".repeat(TEMPLATE_OUTPUT_HEAD_BYTES);
        let output_dir = test_output_dir("multibyte-boundary");
        let artifact_paths = Mutex::new(Vec::new());

        let rendered = truncated_template_command_output(
            output.as_bytes(),
            &PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &artifact_paths,
        )
        .unwrap();

        assert!(rendered.starts_with('é'));
        assert!(rendered.contains("[truncated: showing first 0 of 1 lines; full output: "));
        let _ = fs::remove_dir_all(output_dir);
    }

    fn full_output_path(rendered: &str) -> &str {
        rendered
            .lines()
            .find(|line| line.starts_with("[truncated: "))
            .unwrap()
            .strip_suffix(']')
            .unwrap()
            .rsplit_once("full output: ")
            .unwrap()
            .1
    }

    fn test_output_dir(label: &str) -> PathBuf {
        let random = getrandom::u64().unwrap();
        let path = std::env::temp_dir().join(format!(
            "canon-prompt-template-output-{label}-{}-{random:016x}",
            std::process::id()
        ));
        create_private_dir(&path).unwrap();
        path
    }
}
