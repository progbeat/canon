use crate::config_types::{
    CheckHookCaseOutcome, CheckHookConfig, DEFAULT_CHECK_HOOK_REPAIR_INSTRUCTION,
};
use crate::logs::DiagnosticLogWriter;
use serde_json::{json, Value};
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
    trigger: &str,
    hook_output: &mut dyn Write,
    diagnostic_log: &mut DiagnosticLogWriter,
) -> Result<CheckHookOutcome, String> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    run_check_hooks_with_input(
        root,
        hooks,
        trigger,
        hook_output,
        &mut input,
        Some(diagnostic_log),
    )
}

pub(super) fn run_check_hooks_with_input(
    root: &Path,
    hooks: &[CheckHookConfig],
    trigger: &str,
    hook_output: &mut dyn Write,
    input: &mut dyn BufRead,
    mut diagnostic_log: Option<&mut DiagnosticLogWriter>,
) -> Result<CheckHookOutcome, String> {
    for (index, hook) in hooks.iter().enumerate() {
        write_hook_start_event(diagnostic_log.as_deref_mut(), trigger, index, hook)?;
        let outcome = run_check_hook_with_input(root, hook, hook_output, input)?;
        write_hook_finish_event(diagnostic_log.as_deref_mut(), trigger, index, &outcome)?;
        match outcome {
            CheckHookOutcome::Continue => {}
            blocked @ CheckHookOutcome::Blocked { .. } => return Ok(blocked),
        }
    }
    Ok(CheckHookOutcome::Continue)
}

fn run_check_hook_with_input(
    root: &Path,
    hook: &CheckHookConfig,
    hook_output: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<CheckHookOutcome, String> {
    // Check hooks print at lifecycle points outside expectation interrogation;
    // these bytes are not expectation result entries.
    if let Some(print) = hook.print.as_deref() {
        write_hook_output_fragment(
            hook_output,
            hook_print_line(print).as_bytes(),
            "check hook print",
        )?;
    }
    if let Some(prompt) = hook.input.as_deref() {
        write_hook_output_fragment(hook_output, prompt.as_bytes(), "check hook input prompt")?;
        let key = read_hook_input_line(input)?;
        return Ok(match_hook_cases(hook, &key));
    }
    if let Some(argv) = hook.exec.as_deref() {
        let key = run_hook_exec(root, hook_output, argv)?;
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
    hook_output: &mut dyn Write,
    argv: &[String],
) -> Result<String, String> {
    write_hook_output_fragment(
        hook_output,
        hook_exec_command_line(argv).as_bytes(),
        "check hook exec transcript",
    )?;
    let Some(program) = argv.first() else {
        return Ok(String::new());
    };
    let mut command = Command::new(program);
    command.args(&argv[1..]).current_dir(root);
    let status = command
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

fn write_hook_start_event(
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    trigger: &str,
    index: usize,
    hook: &CheckHookConfig,
) -> Result<(), String> {
    let Some(writer) = diagnostic_log else {
        return Ok(());
    };
    writer
        .write_event(
            "info",
            "check.hook.start",
            &hook_start_event_fields(trigger, index, hook),
        )
        .map_err(|err| err.to_string())
}

fn write_hook_finish_event(
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    trigger: &str,
    index: usize,
    outcome: &CheckHookOutcome,
) -> Result<(), String> {
    let Some(writer) = diagnostic_log else {
        return Ok(());
    };
    let (level, fields) = match outcome {
        CheckHookOutcome::Continue => (
            "info",
            vec![
                ("trigger", json!(trigger)),
                ("index", json!(index)),
                ("outcome", json!("ok")),
            ],
        ),
        CheckHookOutcome::Blocked { repair_instruction } => (
            "warn",
            vec![
                ("trigger", json!(trigger)),
                ("index", json!(index)),
                ("outcome", json!("blocked")),
                ("repairInstruction", json!(repair_instruction)),
            ],
        ),
    };
    writer
        .write_event(level, "check.hook.finish", &fields)
        .map_err(|err| err.to_string())
}

fn hook_start_event_fields(
    trigger: &str,
    index: usize,
    hook: &CheckHookConfig,
) -> Vec<(&'static str, Value)> {
    let mut fields = vec![
        ("trigger", json!(trigger)),
        ("index", json!(index)),
        ("action", json!(hook_action(hook))),
    ];
    if let Some(print) = hook.print.as_deref() {
        fields.push(("print", json!(print)));
    }
    if let Some(input) = hook.input.as_deref() {
        fields.push(("input", json!(input)));
    }
    if let Some(exec) = hook.exec.as_deref() {
        fields.push(("exec", json!(exec)));
    }
    fields
}

fn hook_action(hook: &CheckHookConfig) -> &'static str {
    if hook.input.is_some() {
        "input"
    } else if hook.exec.is_some() {
        "exec"
    } else {
        "print"
    }
}

fn hook_print_line(text: &str) -> String {
    let mut line = text.to_string();
    line.push('\n');
    line
}

fn write_hook_output_fragment(
    output: &mut dyn Write,
    bytes: &[u8],
    description: &str,
) -> Result<(), String> {
    output
        .write_all(bytes)
        .map_err(|err| format!("failed to write {}: {}", description, err))?;
    output
        .flush()
        .map_err(|err| format!("failed to flush {}: {}", description, err))
}

fn hook_exec_command_line(argv: &[String]) -> String {
    let command = argv
        .iter()
        .map(|arg| hook_exec_command_arg(arg))
        .collect::<Vec<_>>()
        .join(" ");
    format!("$ {command}\n")
}

fn hook_exec_command_arg(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':' | '='))
    {
        return arg.to_string();
    }
    let mut quoted = String::from("'");
    for ch in arg.chars() {
        match ch {
            '\'' => quoted.push_str("'\\''"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('\'');
    quoted
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
    use super::{hook_exec_command_line, run_check_hooks_with_input, CheckHookOutcome};
    use crate::config_types::{CheckHookCaseOutcome, CheckHookConfig};
    use std::collections::BTreeMap;
    use std::io::Cursor;
    use std::path::Path;

    // xpec: uY
    #[test]
    fn print_only_hook_continues_and_appends_one_trailing_newline() {
        let hook = CheckHookConfig {
            print: Some("ready\n\n".to_string()),
            input: None,
            exec: None,
            cases: BTreeMap::new(),
        };
        let mut output = Vec::new();
        let mut input = Cursor::new(Vec::<u8>::new());

        let outcome = run_check_hooks_with_input(
            Path::new("."),
            &[hook],
            "on-start",
            &mut output,
            &mut input,
            None,
        )
        .unwrap();

        // xpec: uY
        assert!(matches!(outcome, CheckHookOutcome::Continue));
        // xpec: uY
        assert_eq!(String::from_utf8(output).unwrap(), "ready\n\n\n");
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

        let outcome = run_check_hooks_with_input(
            Path::new("."),
            &[hook],
            "on-start",
            &mut output,
            &mut input,
            None,
        )
        .unwrap();

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

        let outcome = run_check_hooks_with_input(
            Path::new("."),
            &[hook],
            "on-start",
            &mut output,
            &mut input,
            None,
        )
        .unwrap();

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
        let exec = vec![
            test_binary,
            "--exact".to_string(),
            "definitely_not_a_test".to_string(),
            "--quiet".to_string(),
        ];
        let hook = CheckHookConfig {
            print: None,
            input: None,
            exec: Some(exec.clone()),
            cases: cases([("0", CheckHookCaseOutcome::Continue)]),
        };
        let mut output = Vec::new();
        let mut input = Cursor::new(Vec::<u8>::new());

        let outcome = run_check_hooks_with_input(
            Path::new("."),
            &[hook],
            "on-start",
            &mut output,
            &mut input,
            None,
        )
        .unwrap();

        // xpec: uY
        assert!(matches!(outcome, CheckHookOutcome::Continue));
        // xpec: w9
        assert_eq!(
            String::from_utf8(output).unwrap(),
            hook_exec_command_line(&exec)
        );
    }

    // xpec: w9
    #[test]
    fn exec_command_line_uses_command_transcript_format() {
        // xpec: w9
        assert_eq!(
            hook_exec_command_line(&["cargo".to_string(), "test".to_string()]),
            "$ cargo test\n"
        );
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

        let outcome = run_check_hooks_with_input(
            Path::new("."),
            &hooks,
            "on-start",
            &mut output,
            &mut input,
            None,
        )
        .unwrap();

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
