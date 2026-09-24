//! Stage 6 — Documentation & Audit Agent. Runs last, only on modules
//! that passed Verification. Never modifies code or test results.

use crate::llm::LlmClient;
use crate::prompts::{DOCUMENTATION_AGENT, SAFETY_PREAMBLE};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub const REPORT_FILENAME: &str = "06-documentation.md";

pub struct PipelineHistory<'a> {
    pub discovery_json: &'a str,
    pub risk_triage_json: &'a str,
    pub architecture_md: &'a str,
    pub codegen_notes_md: &'a str,
    pub verification_md: &'a str,
    pub compliance_regime: Option<&'a str>,
}

pub fn run(llm: &dyn LlmClient, history: &PipelineHistory, output_dir: &Path) -> Result<PathBuf> {
    let system_prompt = format!("{SAFETY_PREAMBLE}\n{DOCUMENTATION_AGENT}");

    let compliance_line = match history.compliance_regime {
        Some(regime) => format!("NAMED COMPLIANCE REGIME: {regime}\n"),
        None => "NAMED COMPLIANCE REGIME: none provided — omit the Compliance Control Mapping section.\n".to_string(),
    };

    let user_prompt = format!(
        "{compliance_line}\nDISCOVERY OUTPUT:\n{}\n\nRISK & TRIAGE VERDICTS:\n{}\n\nAPPROVED ARCHITECTURE:\n{}\n\nCODE GENERATION NOTES:\n{}\n\nVERIFICATION REPORT:\n{}\n",
        history.discovery_json,
        history.risk_triage_json,
        history.architecture_md,
        history.codegen_notes_md,
        history.verification_md,
    );

    let response = llm
        .chat(&system_prompt, &user_prompt)
        .context("Documentation & Audit Agent call failed")?;

    let output_path = output_dir.join(REPORT_FILENAME);
    fs::create_dir_all(output_dir)?;
    fs::write(&output_path, &response)
        .with_context(|| format!("could not write {}", output_path.display()))?;

    Ok(output_path)
}
