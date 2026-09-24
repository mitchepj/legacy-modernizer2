//! Stage 3 — Architecture & Decoupling Agent, plus the Phase 2 human
//! approval gate.
//!
//! This is the one stage the master design says human judgment should
//! dominate. `run` only ever produces a proposal; nothing in this module
//! auto-approves it. The interactive terminal-prompt gate (the mechanism
//! the user chose) lives in `approve_interactive`, kept separate from
//! `run` so the pipeline itself stays testable without stdin — the demo
//! and test suite use `record_approval` directly to simulate a human
//! having said yes, without blocking on a real terminal.

use crate::llm::LlmClient;
use crate::prompts::{ARCHITECTURE_AGENT, SAFETY_PREAMBLE};
use crate::state::{self, ApprovalRecord};
use anyhow::{Context, Result};
use serde_json::Value;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const REPORT_FILENAME: &str = "03-architecture.md";

pub fn run(
    llm: &dyn LlmClient,
    discovery_output: &Value,
    risk_triage_output: &Value,
    output_dir: &Path,
) -> Result<PathBuf> {
    let system_prompt = format!("{SAFETY_PREAMBLE}\n{ARCHITECTURE_AGENT}");
    let user_prompt = format!(
        "DISCOVERY OUTPUT:\n{}\n\nRISK & TRIAGE VERDICTS:\n{}\n",
        serde_json::to_string_pretty(discovery_output)?,
        serde_json::to_string_pretty(risk_triage_output)?
    );

    let response = llm
        .chat(&system_prompt, &user_prompt)
        .context("Architecture & Decoupling Agent call failed")?;

    let output_path = output_dir.join(REPORT_FILENAME);
    std::fs::create_dir_all(output_dir)?;
    std::fs::write(&output_path, &response)
        .with_context(|| format!("could not write {}", output_path.display()))?;

    Ok(output_path)
}

/// The Phase 2 human gate, enforced as an interactive terminal prompt
/// (the mechanism explicitly chosen for this tool). Blocks on stdin;
/// refuses to record approval on anything but an explicit "yes".
pub fn approve_interactive(architecture_doc_path: &Path, output_dir: &Path, default_approver: Option<&str>) -> Result<bool> {
    println!("\n--- Phase 2 Human Approval Gate ---");
    println!("Architecture proposal: {}", architecture_doc_path.display());
    println!("Per the master design, this stage requires explicit human sign-off before Code Generation may run.");
    print!("Approve this architecture proposal as the basis for Code Generation? [y/N]: ");
    io::stdout().flush().ok();

    let mut answer = String::new();
    io::stdin().read_line(&mut answer).context("failed to read approval response from stdin")?;
    if !answer.trim().eq_ignore_ascii_case("y") && !answer.trim().eq_ignore_ascii_case("yes") {
        println!("Not approved. Code Generation remains blocked.");
        return Ok(false);
    }

    let prompt_label = match default_approver {
        Some(name) => format!("Approver name [{name}]: "),
        None => "Approver name: ".to_string(),
    };
    print!("{prompt_label}");
    io::stdout().flush().ok();
    let mut name = String::new();
    io::stdin().read_line(&mut name).context("failed to read approver name from stdin")?;
    let name = name.trim();
    let approver = if name.is_empty() {
        default_approver.unwrap_or("unspecified").to_string()
    } else {
        name.to_string()
    };

    record_approval(architecture_doc_path, output_dir, &approver, "approved via interactive terminal gate")?;
    println!("Recorded: {approver} approved the architecture proposal.");
    Ok(true)
}

/// Persists an approval record hashing the current architecture doc.
/// Used by both the interactive gate above and by the demo/test harness
/// (to simulate a human approval without stdin).
pub fn record_approval(architecture_doc_path: &Path, output_dir: &Path, approver: &str, notes: &str) -> Result<()> {
    let hash = state::hash_file(architecture_doc_path)?;
    let record = ApprovalRecord {
        approver: approver.to_string(),
        approved_at: state::now_rfc3339(),
        architecture_doc_sha256: hash,
        notes: notes.to_string(),
    };
    record.save(output_dir)
}
