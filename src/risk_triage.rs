//! Stage 2 — Risk & Triage Agent.
//!
//! Consumes Discovery's JSON only (never raw source, per the master
//! design). Scores every module against the immutable Risk Triage
//! Matrix and returns a hard GO / CONDITIONAL / NO-GO verdict per module.

use crate::discovery::parse_json_response;
use crate::llm::LlmClient;
use crate::prompts::{RISK_MATRIX, RISK_TRIAGE_AGENT, SAFETY_PREAMBLE};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const REPORT_FILENAME: &str = "02-risk-triage.json";

pub fn run(llm: &dyn LlmClient, discovery_output: &Value, output_dir: &Path) -> Result<PathBuf> {
    let system_prompt = format!("{SAFETY_PREAMBLE}\n{RISK_MATRIX}\n{RISK_TRIAGE_AGENT}");
    let user_prompt = serde_json::to_string_pretty(discovery_output)
        .context("could not serialize Discovery output for Risk & Triage")?;

    let response = llm
        .chat(&system_prompt, &user_prompt)
        .context("Risk & Triage Agent call failed")?;
    let parsed = parse_json_response(&response)
        .context("Risk & Triage Agent response was not valid JSON matching the required schema")?;

    let output_path = output_dir.join(REPORT_FILENAME);
    std::fs::create_dir_all(output_dir)?;
    std::fs::write(&output_path, serde_json::to_string_pretty(&parsed)?)
        .with_context(|| format!("could not write {}", output_path.display()))?;

    Ok(output_path)
}

/// Convenience accessor used by later stages / the CLI to list only the
/// modules cleared for autonomous transformation (GO or CONDITIONAL) —
/// NO-GO modules never reach Architecture or Code Generation.
pub fn cleared_modules(risk_triage_output: &Value) -> Vec<String> {
    risk_triage_output
        .get("verdicts")
        .and_then(|v| v.as_array())
        .map(|verdicts| {
            verdicts
                .iter()
                .filter(|v| {
                    matches!(
                        v.get("verdict").and_then(|s| s.as_str()),
                        Some("GO") | Some("CONDITIONAL")
                    )
                })
                .filter_map(|v| v.get("module").and_then(|m| m.as_str()).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}
