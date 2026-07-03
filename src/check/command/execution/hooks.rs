use crate::check::command::output::write_stdout_record;
use crate::config_types::{
    CheckHookCaseOutcome, CheckHookConfig, DEFAULT_CHECK_HOOK_REPAIR_INSTRUCTION,
};
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::Command;

pub(super) enum CheckHookOutcome {
    Continue,
    Blocked { repair_instruction: String },
}

pub(super) fn run_check_hooks(
    root: &Path,
    hooks: &[CheckHookConfig],
    result_output: &mut dyn Write,
) -> Result<CheckHookOutcome, String> {
    run_check_hooks_with_input(root, hooks, result_output, &mut io::stdin().lock())
}

pub(super) fn run_check_hooks_with_input(
    root: &Path,
    hooks: &[CheckHookConfig],
    result_output: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<CheckHookOutcome, String> {
    for hook in hooks {
        match run_check_hook_with_input(root, hook, result_output, input)? {
            CheckHookOutcome::Continue => {}
            blocked @ CheckHookOutcome::Blocked { .. } => return Ok(blocked),
        }
    }
    Ok(CheckHookOutcome::Continue)
}

fn run_check_hook_with_input(
    root: &Path,
    hook: &CheckHookConfig,
    result_output: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<CheckHookOutcome, String> {
    if let Some(print) = hook.print.as_deref() {
        write_stdout_record(
            result_output,
            hook_print_line(print).as_bytes(),
            "check hook output",
        )?;
    }
    if let Some(prompt) = hook.input.as_deref() {
        write_stdout_record(result_output, prompt.as_bytes(), "check hook input prompt")?;
        let key = read_hook_input_line(input)?;
        return Ok(match_hook_cases(hook, &key));
    }
    if let Some(argv) = hook.exec.as_deref() {
        let key = run_hook_exec(root, result_output, argv)?;
        return Ok(match_hook_cases(hook, &key));
    }
    Ok(CheckHookOutcome::Continue)
}

fn read_hook_input_line(input: &mut dyn BufRead) -> Result<String, String> {
    let mut line = String::new();
    input
        .read_line(&mut line)
        .map_err(|err| format!("failed to read check hook input: {}", err))?;
    trim_stdin_line_ending(&mut line);
    Ok(line)
}

fn run_hook_exec(
    root: &Path,
    result_output: &mut dyn Write,
    argv: &[String],
) -> Result<String, String> {
    result_output
        .flush()
        .map_err(|err| format!("failed to flush check hook output before exec: {}", err))?;
    let Some(program) = argv.first() else {
        return Ok(String::new());
    };
    let status = Command::new(program)
        .args(&argv[1..])
        .current_dir(root)
        .status()
        .map_err(|err| format!("failed to run check hook exec {}: {}", program, err))?;
    Ok(status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "<terminated-by-signal>".to_string()))
}

fn match_hook_cases(hook: &CheckHookConfig, key: &str) -> CheckHookOutcome {
    let outcome = hook.cases.get(key).or_else(|| hook.cases.get("_"));
    match outcome {
        Some(CheckHookCaseOutcome::Continue) => CheckHookOutcome::Continue,
        Some(CheckHookCaseOutcome::Block { repair_instruction }) => CheckHookOutcome::Blocked {
            repair_instruction: repair_instruction.clone(),
        },
        None => CheckHookOutcome::Blocked {
            repair_instruction: DEFAULT_CHECK_HOOK_REPAIR_INSTRUCTION.to_string(),
        },
    }
}

fn hook_print_line(text: &str) -> String {
    let mut line = text.trim_end_matches(['\r', '\n']).to_string();
    line.push('\n');
    line
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
    use super::{run_check_hooks_with_input, CheckHookOutcome};
    use crate::config_types::{CheckHookCaseOutcome, CheckHookConfig};
    use std::collections::BTreeMap;
    use std::io::Cursor;
    use std::path::Path;

    // xpec: uY
    #[test]
    fn print_only_hook_continues_and_prints_one_trailing_newline() {
        let hook = CheckHookConfig {
            print: Some("ready\n\n".to_string()),
            input: None,
            exec: None,
            cases: BTreeMap::new(),
        };
        let mut output = Vec::new();
        let mut input = Cursor::new(Vec::<u8>::new());

        let outcome =
            run_check_hooks_with_input(Path::new("."), &[hook], &mut output, &mut input).unwrap();

        // xpec: uY
        assert!(matches!(outcome, CheckHookOutcome::Continue));
        // xpec: uY
        assert_eq!(String::from_utf8(output).unwrap(), "ready\n");
    }

    // xpec: uY
    #[test]
    fn input_hook_prints_prompt_and_trims_only_line_ending() {
        let hook = CheckHookConfig {
            print: None,
            input: Some("type pass: ".to_string()),
            exec: None,
            cases: cases([("pass ", CheckHookCaseOutcome::Continue)]),
        };
        let mut output = Vec::new();
        let mut input = Cursor::new(b"pass \r\n".to_vec());

        let outcome =
            run_check_hooks_with_input(Path::new("."), &[hook], &mut output, &mut input).unwrap();

        // xpec: uY
        assert!(matches!(outcome, CheckHookOutcome::Continue));
        // xpec: uY
        assert_eq!(String::from_utf8(output).unwrap(), "type pass: ");
    }

    // xpec: uY
    #[test]
    fn missing_case_blocks_with_default_repair_instruction() {
        let hook = CheckHookConfig {
            print: None,
            input: Some("type pass: ".to_string()),
            exec: None,
            cases: cases([("pass", CheckHookCaseOutcome::Continue)]),
        };
        let mut output = Vec::new();
        let mut input = Cursor::new(b"fail\n".to_vec());

        let outcome =
            run_check_hooks_with_input(Path::new("."), &[hook], &mut output, &mut input).unwrap();

        match outcome {
            CheckHookOutcome::Blocked { repair_instruction } => {
                // xpec: uY
                assert_eq!(
                    repair_instruction,
                    crate::config_types::DEFAULT_CHECK_HOOK_REPAIR_INSTRUCTION
                );
            }
            CheckHookOutcome::Continue => panic!("expected hook to block"),
        }
    }

    // xpec: uY
    #[test]
    fn exec_hook_matches_process_exit_code() {
        let test_binary = std::env::current_exe().unwrap().display().to_string();
        let hook = CheckHookConfig {
            print: None,
            input: None,
            exec: Some(vec![
                test_binary,
                "--exact".to_string(),
                "definitely_not_a_test".to_string(),
                "--quiet".to_string(),
            ]),
            cases: cases([("0", CheckHookCaseOutcome::Continue)]),
        };
        let mut output = Vec::new();
        let mut input = Cursor::new(Vec::<u8>::new());

        let outcome =
            run_check_hooks_with_input(Path::new("."), &[hook], &mut output, &mut input).unwrap();

        // xpec: uY
        assert!(matches!(outcome, CheckHookOutcome::Continue));
    }

    // xpec: uY
    #[test]
    fn hook_list_runs_in_order_and_stops_on_block() {
        let hooks = vec![
            CheckHookConfig {
                print: Some("first".to_string()),
                input: None,
                exec: None,
                cases: BTreeMap::new(),
            },
            CheckHookConfig {
                print: Some("second".to_string()),
                input: Some("prompt".to_string()),
                exec: None,
                cases: cases([(
                    "_",
                    CheckHookCaseOutcome::Block {
                        repair_instruction: "second repair".to_string(),
                    },
                )]),
            },
            CheckHookConfig {
                print: Some("third".to_string()),
                input: None,
                exec: None,
                cases: BTreeMap::new(),
            },
        ];
        let mut output = Vec::new();
        let mut input = Cursor::new(b"fail\n".to_vec());

        let outcome =
            run_check_hooks_with_input(Path::new("."), &hooks, &mut output, &mut input).unwrap();

        match outcome {
            CheckHookOutcome::Blocked { repair_instruction } => {
                // xpec: uY
                assert_eq!(repair_instruction, "second repair");
            }
            CheckHookOutcome::Continue => panic!("expected hook to block"),
        }
        // xpec: uY
        assert_eq!(String::from_utf8(output).unwrap(), "first\nsecond\nprompt");
    }

    fn cases<const N: usize>(
        cases: [(&str, CheckHookCaseOutcome); N],
    ) -> BTreeMap<String, CheckHookCaseOutcome> {
        cases
            .into_iter()
            .map(|(key, outcome)| (key.to_string(), outcome))
            .collect()
    }
}
