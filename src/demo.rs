//! `demo` — an end-to-end, fully offline run of all six pipeline stages
//! against a tiny embedded legacy C snippet, using `MockLlmClient`
//! instead of a real model server.
//!
//! This exists so that cloning this repo and running one command (or
//! `cargo test`) proves the *orchestration* works — sequencing, the
//! Phase 2 approval-hash gate, fenced-code-block extraction, report
//! generation — independent of any particular self-hosted model's
//! output quality, and with zero network access required. It is the
//! "prototype of the agents" harness: real prompts (`prompts.rs`, byte
//! for byte what a live engagement uses), scripted responses standing in
//! for the model.
//!
//! `run_full_demo` is called by both the `legacy-modernizer demo` CLI
//! subcommand and by `tests/prototype_demo.rs`, so the same code path is
//! exercised by a person running it by hand and by CI.

use crate::llm::MockLlmClient;
use crate::state::LedgerEntry;
use crate::{architecture, codegen, discovery, document, risk_triage, state, verify};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// A minimal, self-contained, obviously-synthetic "legacy" snippet —
/// small enough to need only one Discovery batch, with just enough
/// structure (a global counter, a branch, an external call) to give
/// every stage something real to talk about.
const SAMPLE_LEGACY_SOURCE: &str = r#"/* sample_counter.c — synthetic fixture for the `demo` command only. */
#include <stdio.h>

static int g_counter = 0;

int increment_and_check(int threshold) {
    g_counter = g_counter + 1;
    if (g_counter > threshold) {
        printf("threshold exceeded: %d\n", g_counter);
        return 1;
    }
    return 0;
}
"#;

pub struct DemoSummary {
    pub output_dir: PathBuf,
    pub discovery_report: PathBuf,
    pub risk_triage_report: PathBuf,
    pub architecture_report: PathBuf,
    pub codegen_report: PathBuf,
    pub generated_files: Vec<PathBuf>,
    pub verification_report: PathBuf,
    pub verdict: verify::Verdict,
    pub documentation_report: PathBuf,
}

/// Scripted responses, one per LLM call, in the exact order the pipeline
/// makes them: Discovery (1 batch) -> Risk & Triage -> Architecture ->
/// Code Generation -> Verification -> Documentation.
fn scripted_responses() -> Vec<String> {
    vec![
        // 1. Discovery
        r#"{
  "modules": [
    {
      "name": "sample_counter",
      "language": "C",
      "files": ["sample_counter.c"],
      "loc_estimate": 12,
      "call_graph_summary": "increment_and_check() is the sole entry point; calls printf() when threshold exceeded.",
      "shared_state_touched": ["g_counter"],
      "external_dependencies": ["stdio.h:printf"],
      "control_flow_shape": "linear",
      "complexity_signal": "1 branch, 1 global, fan-in unknown from this context",
      "confidence": "INFERRED",
      "test_coverage_evidence": "unknown - no test evidence found in the provided context",
      "flags": []
    }
  ],
  "notes_for_human_review": ["Synthetic demo fixture - not real legacy code."]
}"#.to_string(),
        // 2. Risk & Triage
        r#"{
  "verdicts": [
    {
      "module": "sample_counter",
      "verdict": "GO",
      "worst_dimension": "test coverage",
      "rationale": "Standalone module, linear control flow, no hardware/OS proximity; downgraded one tier for INFERRED confidence but still clears GO.",
      "mitigation_required": null,
      "redline_evidence": null
    }
  ],
  "pilot_recommendation": "sample_counter",
  "pilot_rationale": "Only module in this demo batch; trivially decoupled."
}"#.to_string(),
        // 3. Architecture
        "## Proposed Service/Module Boundaries\n\n`sample_counter` becomes a single owned-state `CounterService` struct - no free-standing global.\n\n| Legacy module | Target boundary |\n|---|---|\n| sample_counter.c | CounterService (library) |\n\n## Interface Contracts\n\n`CounterService::increment_and_check(&mut self, threshold: i32) -> bool`\n\n## Concurrency & State-Ownership Design\n\n`g_counter` becomes a private field on `CounterService`, replacing the legacy file-scope global with instance-owned state.\n\n## Trade-Off Notes for Human Reviewers\n\n| decision | option chosen | why | what would change the call |\n|---|---|---|---|\n| logging | return bool, let caller log | keeps service pure/testable | if callers need the exact printf format preserved, reintroduce it explicitly |\n\n## Human Gate\nThis proposal requires human approval before Code Generation may proceed.".to_string(),
        // 4. Code Generation
        "```rust file=counter_service.rs\npub struct CounterService {\n    counter: i32,\n}\n\nimpl CounterService {\n    pub fn new() -> Self {\n        CounterService { counter: 0 }\n    }\n\n    pub fn increment_and_check(&mut self, threshold: i32) -> bool {\n        self.counter += 1;\n        self.counter > threshold\n    }\n}\n```\n\n## Deviations From a Literal Translation\n- Replaced the legacy `printf` side effect with a returned `bool`; the architecture proposal's Trade-Off Notes flag this explicitly.\n\n## Ambiguities Flagged for Human Clarification\nNone.".to_string(),
        // 5. Verification
        "## Test Suite\n```rust file=counter_service_tests.rs\n#[test]\nfn increments_and_signals_past_threshold() {\n    let mut c = CounterService::new();\n    assert!(!c.increment_and_check(2));\n    assert!(!c.increment_and_check(2));\n    assert!(c.increment_and_check(2));\n}\n```\n\n## Behavioral Divergence Findings\nNone identified beyond the disclosed printf-to-bool change.\n\n## Static-Analysis-Style Findings\nNone.\n\n## Performance Note\nNo benchmark run - no execution environment wired into this pass.\n\n## Verdict\nVerdict: PASS".to_string(),
        // 6. Documentation
        "## ADRs\n### sample_counter -> CounterService\nMigrated from C to Rust. Global `g_counter` replaced with instance-owned state per the approved architecture.\n\n## Traceability Matrix\n| Legacy | New location |\n|---|---|\n| increment_and_check() | CounterService::increment_and_check() |\n\n## Program Status Summary\n| Modules processed | GO | CONDITIONAL | NO-GO | Migrated |\n|---|---|---|---|---|\n| 1 | 1 | 0 | 0 | 1 |".to_string(),
    ]
}

/// Runs the full six-stage pipeline against the embedded sample source,
/// writing every numbered report and generated file under `output_dir`.
/// Returns paths to everything produced, for the CLI to print or for a
/// test to assert against.
pub fn run_full_demo(output_dir: &Path) -> Result<DemoSummary> {
    std::fs::create_dir_all(output_dir).context("could not create demo output directory")?;

    // Write the embedded sample into its own source subdirectory so
    // Discovery walks a real directory, exactly like a live engagement.
    let source_dir = output_dir.join("sample-legacy-source");
    std::fs::create_dir_all(&source_dir)?;
    std::fs::write(source_dir.join("sample_counter.c"), SAMPLE_LEGACY_SOURCE)?;

    let llm = MockLlmClient::new(scripted_responses());

    let t0 = state::now_rfc3339();
    let discovery_report = discovery::run(&llm, &source_dir, output_dir, &[], 2000)?;
    state::RunLedger::append(output_dir, LedgerEntry {
        stage: "discovery".to_string(),
        started_at: t0.clone(),
        finished_at: state::now_rfc3339(),
        status: "ok".to_string(),
        detail: format!("wrote {}", discovery_report.display()),
    })?;
    let discovery_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&discovery_report)?)?;

    let risk_triage_report = risk_triage::run(&llm, &discovery_json, output_dir)?;
    let risk_triage_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&risk_triage_report)?)?;

    let architecture_report = architecture::run(&llm, &discovery_json, &risk_triage_json, output_dir)?;
    let architecture_md = std::fs::read_to_string(&architecture_report)?;

    // Simulate the Phase 2 human gate having been approved, without
    // blocking on stdin (that's `architecture::approve_interactive`,
    // exercised by the real CLI, not by this offline demo/test path).
    architecture::record_approval(&architecture_report, output_dir, "demo-harness", "auto-approved by `demo`/test harness, not a real human sign-off")?;
    anyhow::ensure!(
        state::ApprovalRecord::is_valid_for(output_dir, &architecture_report)?,
        "demo approval record did not validate against the architecture doc hash - gate logic bug"
    );

    let codegen_result = codegen::run(&llm, &architecture_md, SAMPLE_LEGACY_SOURCE, output_dir)?;

    let verification_result = verify::run(
        &llm,
        SAMPLE_LEGACY_SOURCE,
        &codegen_result.files_written.iter()
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .collect::<Vec<_>>()
            .join("\n"),
        &architecture_md,
        output_dir,
    )?;

    let codegen_notes_md = std::fs::read_to_string(&codegen_result.report_path)?;
    let verification_md = std::fs::read_to_string(&verification_result.report_path)?;
    let risk_triage_json_str = std::fs::read_to_string(&risk_triage_report)?;
    let discovery_json_str = std::fs::read_to_string(&discovery_report)?;

    let history = document::PipelineHistory {
        discovery_json: &discovery_json_str,
        risk_triage_json: &risk_triage_json_str,
        architecture_md: &architecture_md,
        codegen_notes_md: &codegen_notes_md,
        verification_md: &verification_md,
        compliance_regime: None,
    };
    let documentation_report = document::run(&llm, &history, output_dir)?;

    Ok(DemoSummary {
        output_dir: output_dir.to_path_buf(),
        discovery_report,
        risk_triage_report,
        architecture_report,
        codegen_report: codegen_result.report_path,
        generated_files: codegen_result.files_written,
        verification_report: verification_result.report_path,
        verdict: verification_result.verdict,
        documentation_report,
    })
}
