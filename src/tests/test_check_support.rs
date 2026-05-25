use super::*;

pub(crate) fn check_config_yaml() -> &'static str {
    r#"
version: 1
agent:
  model:
    primary: gpt-5.4-mini
    fallbacks:
      - gpt-5.3-codex-spark
  thinking: medium
  ignore:
    - "target/**"
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
  - q: "Second?"
    a: "no"
"#
}

pub(crate) fn parse_check_config(yaml: &str) -> Result<CheckConfig, String> {
    parse_check_config_content(Path::new(".canon/check.yml"), yaml)
}

pub(crate) struct FakeRunner {
    pub(crate) answers: VecDeque<Result<String, EvaluatorError>>,
    pub(crate) turn_usages: VecDeque<Option<EvaluatorTurnUsage>>,
    pub(crate) last_turn_usage: Option<EvaluatorTurnUsage>,
    pub(crate) retired_sessions: VecDeque<Vec<String>>,
    pub(crate) prompts: Vec<String>,
    pub(crate) sessions: Vec<String>,
    pub(crate) ask_models: Vec<Option<String>>,
    pub(crate) ask_thinking: Vec<String>,
    pub(crate) start_instructions: Vec<String>,
    pub(crate) start_roots: Vec<PathBuf>,
    pub(crate) start_ignores: Vec<Vec<String>>,
    pub(crate) start_models: Vec<Option<String>>,
    pub(crate) start_thinking: Vec<String>,
    pub(crate) start_plugins: Vec<Vec<String>>,
    pub(crate) start_scopes: Vec<Vec<String>>,
    pub(crate) starts: usize,
}

impl FakeRunner {
    pub(crate) fn new(answers: &[&str]) -> FakeRunner {
        FakeRunner {
            answers: answers
                .iter()
                .map(|answer| Ok((*answer).to_string()))
                .collect(),
            turn_usages: VecDeque::new(),
            last_turn_usage: None,
            retired_sessions: VecDeque::new(),
            prompts: Vec::new(),
            sessions: Vec::new(),
            ask_models: Vec::new(),
            ask_thinking: Vec::new(),
            start_instructions: Vec::new(),
            start_roots: Vec::new(),
            start_ignores: Vec::new(),
            start_models: Vec::new(),
            start_thinking: Vec::new(),
            start_plugins: Vec::new(),
            start_scopes: Vec::new(),
            starts: 0,
        }
    }

    pub(crate) fn new_results(answers: Vec<Result<&str, EvaluatorError>>) -> FakeRunner {
        FakeRunner {
            answers: answers
                .into_iter()
                .map(|answer| answer.map(str::to_string))
                .collect(),
            turn_usages: VecDeque::new(),
            last_turn_usage: None,
            retired_sessions: VecDeque::new(),
            prompts: Vec::new(),
            sessions: Vec::new(),
            ask_models: Vec::new(),
            ask_thinking: Vec::new(),
            start_instructions: Vec::new(),
            start_roots: Vec::new(),
            start_ignores: Vec::new(),
            start_models: Vec::new(),
            start_thinking: Vec::new(),
            start_plugins: Vec::new(),
            start_scopes: Vec::new(),
            starts: 0,
        }
    }
}

impl EvaluatorRunner for FakeRunner {
    fn start_session(
        &mut self,
        session_cwd: &Path,
        instructions: &str,
        agent: &AgentConfig,
        model: Option<&str>,
        thinking: &str,
        scope: &[String],
    ) -> Result<String, EvaluatorError> {
        self.starts += 1;
        self.start_instructions.push(instructions.to_string());
        self.start_roots.push(session_cwd.to_path_buf());
        self.start_ignores.push(effective_ignore_patterns(agent));
        self.start_models
            .push(model.or(agent.model.primary.as_deref()).map(str::to_string));
        self.start_thinking.push(thinking.to_string());
        self.start_plugins.push(agent.plugins.clone());
        self.start_scopes.push(scope.to_vec());
        Ok(format!("session-{}", self.starts))
    }

    fn ask(
        &mut self,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
        thinking: &str,
    ) -> Result<String, EvaluatorError> {
        self.sessions.push(session_id.to_string());
        self.prompts.push(prompt.to_string());
        self.ask_models.push(model.map(str::to_string));
        self.ask_thinking.push(thinking.to_string());
        self.last_turn_usage = self
            .turn_usages
            .pop_front()
            .unwrap_or_else(|| Some(fake_turn_usage(session_id, self.prompts.len())));
        if let Some(turn_usage) = &self.last_turn_usage {
            if thread_reuse_policy_should_retire(carryover_tokens(turn_usage.usage)) {
                self.retired_sessions
                    .push_back(vec![turn_usage.thread_id.clone()]);
            }
        }
        self.answers
            .pop_front()
            .unwrap_or_else(|| Err("fake runner has no answer".into()))
    }

    fn take_last_turn_usage(&mut self) -> Option<EvaluatorTurnUsage> {
        self.last_turn_usage.take()
    }

    fn take_retired_sessions(&mut self) -> Vec<String> {
        self.retired_sessions.pop_front().unwrap_or_default()
    }
}

fn fake_turn_usage(session_id: &str, turn_number: usize) -> EvaluatorTurnUsage {
    EvaluatorTurnUsage {
        thread_id: session_id.to_string(),
        turn_id: format!("turn-{}", turn_number),
        usage: TokenUsage {
            total_tokens: 1,
            input_tokens: 1,
            cached_input_tokens: 0,
            output_tokens: 0,
            reasoning_output_tokens: 0,
        },
        token_usage_updates: Vec::new(),
        context_compaction_events: Vec::new(),
    }
}

pub(crate) struct FlushCountingWriter {
    pub(crate) bytes: Vec<u8>,
    pub(crate) flushes: usize,
}

impl FlushCountingWriter {
    pub(crate) fn new() -> FlushCountingWriter {
        FlushCountingWriter {
            bytes: Vec::new(),
            flushes: 0,
        }
    }
}

impl std::io::Write for FlushCountingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

pub(crate) fn check_options(
    config: &CheckConfig,
    selectors: &[&str],
    stop_after_non_pass: bool,
    ignore_cache: bool,
) -> CheckOptions {
    let selectors = selectors
        .iter()
        .map(|selector| test_selector(config, selector))
        .collect::<Vec<_>>();
    let selected = select_expectations(
        config,
        &selectors.iter().map(OsString::from).collect::<Vec<_>>(),
    )
    .unwrap();
    let non_selected = initial_non_selected_expectations(config, &selected).unwrap();
    let skipped = config.expectations.len().saturating_sub(selected.len());
    CheckOptions {
        selected,
        non_selected,
        selectors_provided: !selectors.is_empty(),
        skipped,
        check_all: !stop_after_non_pass,
        ignore_cache,
        ignore_cooldown: false,
        break_after_tokens: None,
    }
}

pub(crate) fn test_selector(config: &CheckConfig, selector: &str) -> String {
    if let Ok(number) = selector.parse::<usize>() {
        if let Some(expectation) = config.expectations.get(number.saturating_sub(1)) {
            return expectation_id(&expectation.q, &expectation.a);
        }
    }
    selector.to_string()
}

pub(crate) fn answer(answer: &str, evidence: &str, scope: &[&str]) -> String {
    let evidence = cited_evidence(evidence, scope);
    serde_json::to_string(&json!({
        "answer": answer,
        "evidence": evidence,
        "scope": scope,
    }))
    .unwrap()
}

fn cited_evidence(evidence: &str, scope: &[&str]) -> String {
    if evidence.trim().is_empty() || evidence.contains('`') {
        return evidence.to_string();
    }
    let citation = scope
        .iter()
        .copied()
        .find(|path| *path != ".")
        .map(evidence_file_ref)
        .unwrap_or_else(|| "README.md".to_string());
    format!("`{citation}`: {evidence}")
}

fn evidence_file_ref(path: &str) -> String {
    let path = path.trim_end_matches('/');
    if path
        .rsplit('/')
        .next()
        .is_some_and(|basename| basename.contains('.'))
    {
        path.to_string()
    } else {
        format!("{path}/main.rs")
    }
}

fn check_result_from_label(label: &str) -> CheckResult {
    match label {
        RESULT_PASS => CheckResult::Pass,
        RESULT_FAIL => CheckResult::Fail,
        _ => panic!("unsupported check result label: {}", label),
    }
}

pub(crate) fn sample_record(number: usize, result: &str) -> CheckRecord {
    let prompt = "Question?".to_string();
    let expected = "yes".to_string();
    let id = expectation_id(&prompt, &expected);
    CheckRecord {
        timestamp: "1970-01-01T00:00:00Z".to_string(),
        id: id.clone(),
        display_id: id[..1].to_string(),
        number,
        result: check_result_from_label(result),
        prompt: Some(prompt),
        expected: Some(expected),
        observed: if result == "pass" { "yes" } else { "no" }.to_string(),
        evidence: "README.md has evidence".to_string(),
        scope: vec![".".to_string()],
        visible_tree_oid: "AAAAAAAAAAAAAAAAAAAA".to_string(),
        cache_key: None,
    }
}

pub(crate) fn expectation_record(
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    result: &str,
    observed: &str,
    visible_tree_oid: String,
) -> CheckRecord {
    CheckRecord {
        timestamp: "1970-01-01T00:00:00Z".to_string(),
        id: expectation.id.clone(),
        display_id: expectation.display_id.clone(),
        number: expectation.number,
        result: check_result_from_label(result),
        prompt: Some(expectation.q.clone()),
        expected: Some(expectation.a.clone()),
        observed: observed.to_string(),
        evidence: "cached answer".to_string(),
        scope: full_scope(),
        visible_tree_oid,
        cache_key: Some(history_cache_key(agent, expectation)),
    }
}
