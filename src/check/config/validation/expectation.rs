use super::agent::validate_resolved_agent_config;
use crate::check::core::{
    escape_inline_text as escape_config_error_block_text, matches_answer_pattern, ANSWER_PATTERN,
    ERROR_INVALID_QUESTION, ERROR_SCOPE_TOO_NARROW, INTERNAL_ERROR_UNPARSABLE,
};
use crate::check::minimal_unique_expectation_prefix;
use crate::config_types::{AgentConfig, CheckConfig, Expectation};
use crate::hash::expectation_id;
use std::collections::{BTreeSet, HashSet};

#[derive(Hash, PartialEq, Eq)]
struct ResolvedAgentValidationKey<'a> {
    models: &'a [String],
    thinking: &'a str,
    plugins: &'a [String],
}

#[derive(Default)]
struct ResolvedAgentValidationCache<'a> {
    validated: HashSet<ResolvedAgentValidationKey<'a>>,
}

impl<'a> ResolvedAgentValidationCache<'a> {
    fn validate(&mut self, agent: &'a AgentConfig, label: &str) -> Result<(), String> {
        let key = ResolvedAgentValidationKey {
            models: &agent.models,
            thinking: &agent.thinking,
            plugins: &agent.plugins,
        };
        if self.validated.insert(key) {
            validate_resolved_agent_config(agent, label)?;
        }
        Ok(())
    }
}

pub(crate) fn validate_check_config(config: &CheckConfig) -> Result<(), String> {
    validate_config(config, false)
}

pub(crate) fn validate_ask_config(config: &CheckConfig) -> Result<(), String> {
    let [expectation] = config.expectations.as_slice() else {
        return Err("ask config must contain exactly one temporary expectation".to_string());
    };
    if expectation.to != crate::config_types::ExpectationTo::Agent {
        return Err("ask temporary expectation must address the agent".to_string());
    }
    if !expectation.a.is_empty() {
        return Err("ask temporary expectation must have an empty expected answer".to_string());
    }
    validate_config(config, true)
}

fn validate_config(config: &CheckConfig, allow_empty_expected_answer: bool) -> Result<(), String> {
    if config.version != 1 {
        return Err("check.yml version must be 1".to_string());
    }
    let mut agent_validation = ResolvedAgentValidationCache::default();
    agent_validation.validate(&config.agent, "config agent")?;
    validate_unique_expectation_identity_inputs(config)?;
    for (index, expectation) in config.expectations.iter().enumerate() {
        let number = index + 1;
        // Expected answers cannot collide with either evaluator schema errors
        // or Canon's internal unparsable-response marker.
        if matches!(
            expectation.a.as_str(),
            ERROR_SCOPE_TOO_NARROW | ERROR_INVALID_QUESTION | INTERNAL_ERROR_UNPARSABLE
        ) {
            return Err(render_expectation_validation_error(
                &expectation_display_id(config, index),
                &expectation.q,
                "expected answer must not be an evaluator error token",
                &format!(
                    "configured expected answer is `{}`",
                    escape_config_error_block_text(&expectation.a)
                ),
            ));
        }
        if !(allow_empty_expected_answer && expectation.a.is_empty()) {
            // [MH,a] Scalar-to-string normalization and answer-domain
            // validation are separate requirements. A YAML number such as
            // 1.5 and a YAML string "1.5" both reach this boundary as the same
            // String and both fail the schema's answer pattern.
            validate_expected_answer_matches_interrogation_response_schema_answer_pattern(
                config,
                index,
                expectation,
            )?;
        }
        agent_validation.validate(&expectation.agent, &format!("expectation {}", number))?;
    }
    Ok(())
}

fn validate_expected_answer_matches_interrogation_response_schema_answer_pattern(
    config: &CheckConfig,
    index: usize,
    expectation: &Expectation,
) -> Result<(), String> {
    if matches_answer_pattern(&expectation.a) {
        return Ok(());
    }
    Err(render_expectation_validation_error(
        &expectation_display_id(config, index),
        &expectation.q,
        "invalid-expected-answer",
        &format!(
            "configured expected answer `{}` does not match answer pattern {}",
            escape_config_error_block_text(&expectation.a),
            ANSWER_PATTERN
        ),
    ))
}

fn render_expectation_validation_error(
    display_id: &str,
    question: &str,
    error: &str,
    evidence: &str,
) -> String {
    // xpec: RC
    assert_ne!(
        error, ERROR_SCOPE_TOO_NARROW,
        "public expectation error blocks must not expose ScopeTooNarrow"
    );
    format!(
        "{}. ERROR\n{}\nError: {}\nEvidence: {}",
        display_id,
        escape_config_error_block_text(question),
        escape_config_error_block_text(error),
        escape_config_error_block_text(evidence)
    )
}

fn expectation_ids(config: &CheckConfig) -> Vec<String> {
    config
        .expectations
        .iter()
        .map(|expectation| {
            expectation_id(
                &expectation.q,
                expectation.to.as_str(),
                &expectation.a,
                &expectation.question_context,
            )
        })
        .collect()
}

fn validate_unique_expectation_identity_inputs(config: &CheckConfig) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for expectation in &config.expectations {
        let identity_inputs = (
            expectation.q.as_str(),
            expectation.to.as_str(),
            expectation.a.as_str(),
            expectation.question_context.as_str(),
        );
        if !seen.insert(identity_inputs) {
            return Err(format!(
                "duplicate expectation ID: {}",
                expectation_id(
                    &expectation.q,
                    expectation.to.as_str(),
                    &expectation.a,
                    &expectation.question_context,
                )
            ));
        }
    }
    Ok(())
}

fn expectation_display_id(config: &CheckConfig, index: usize) -> String {
    let ids = expectation_ids(config);
    expectation_display_ids(&ids)
        .into_iter()
        .nth(index)
        .expect("expectation display ID index must be collected")
}

fn expectation_display_ids(ids: &[String]) -> Vec<String> {
    ids.iter()
        .map(|id| minimal_unique_expectation_prefix(id, ids).unwrap_or_else(|| id.clone()))
        .collect()
}
