use crate::check::command::output::write_stdout_record;
use crate::config_types::CheckHookConfig;
use std::io::{self, BufRead, Write};

pub(super) enum CheckHookOutcome {
    Continue,
    Blocked { repair_instruction: String },
}

pub(super) fn run_check_hook(
    hook: Option<&CheckHookConfig>,
    result_output: &mut dyn Write,
) -> Result<CheckHookOutcome, String> {
    run_check_hook_with_input(hook, result_output, &mut io::stdin().lock())
}

pub(super) fn run_check_hook_with_input(
    hook: Option<&CheckHookConfig>,
    result_output: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<CheckHookOutcome, String> {
    let Some(hook) = hook else {
        return Ok(CheckHookOutcome::Continue);
    };
    if hook.print.is_empty() {
        return Ok(CheckHookOutcome::Continue);
    }
    write_stdout_record(result_output, hook.print.as_bytes(), "check hook output")?;
    if hook_confirmed(hook, input)? {
        return Ok(CheckHookOutcome::Continue);
    }
    Ok(CheckHookOutcome::Blocked {
        repair_instruction: hook.repair_instruction.clone(),
    })
}

fn hook_confirmed(hook: &CheckHookConfig, input: &mut dyn BufRead) -> Result<bool, String> {
    let Some(confirm) = hook.confirm.as_deref() else {
        return Ok(true);
    };
    let mut line = String::new();
    input
        .read_line(&mut line)
        .map_err(|err| format!("failed to read check hook confirmation: {}", err))?;
    trim_stdin_line_ending(&mut line);
    Ok(line == confirm)
}

fn trim_stdin_line_ending(line: &mut String) {
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{run_check_hook_with_input, CheckHookOutcome};
    use crate::config_types::CheckHookConfig;
    use std::io::Cursor;

    #[test]
    fn blank_print_skips_confirmation() {
        let hook = CheckHookConfig {
            print: String::new(),
            confirm: Some("pass".to_string()),
            repair_instruction: "repair".to_string(),
        };
        let mut output = Vec::new();
        let mut input = Cursor::new(Vec::<u8>::new());

        let outcome = run_check_hook_with_input(Some(&hook), &mut output, &mut input).unwrap();

        assert!(matches!(outcome, CheckHookOutcome::Continue));
        assert!(output.is_empty());
    }

    #[test]
    fn matching_confirmation_continues() {
        let hook = CheckHookConfig {
            print: "type pass\n".to_string(),
            confirm: Some("pass".to_string()),
            repair_instruction: "repair".to_string(),
        };
        let mut output = Vec::new();
        let mut input = Cursor::new(b"pass\n".to_vec());

        let outcome = run_check_hook_with_input(Some(&hook), &mut output, &mut input).unwrap();

        assert!(matches!(outcome, CheckHookOutcome::Continue));
        assert_eq!(String::from_utf8(output).unwrap(), "type pass\n");
    }

    #[test]
    fn mismatched_confirmation_blocks() {
        let hook = CheckHookConfig {
            print: "type pass\n".to_string(),
            confirm: Some("pass".to_string()),
            repair_instruction: "repair".to_string(),
        };
        let mut output = Vec::new();
        let mut input = Cursor::new(b"fail\n".to_vec());

        let outcome = run_check_hook_with_input(Some(&hook), &mut output, &mut input).unwrap();

        match outcome {
            CheckHookOutcome::Blocked { repair_instruction } => {
                assert_eq!(repair_instruction, "repair");
            }
            CheckHookOutcome::Continue => panic!("expected hook to block"),
        }
    }
}
