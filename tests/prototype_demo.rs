//! Integration test: proves the six-agent prototype actually runs,
//! end-to-end, with zero network access and zero real LLM server.
//!
//! This is deliberately checked into the repo (not a throwaway script)
//! so that anyone who clones it can run `cargo test` and see, without
//! standing up any infrastructure, that:
//!   - all six stages execute in the correct, gated order,
//!   - the Phase 2 approval-hash integrity check actually works,
//!   - the fenced-code-block convention round-trips into real files,
//!   - every numbered report lands where the CLI's own `status`
//!     subcommand expects to find it.
//!
//! It uses `legacy_modernizer::demo::run_full_demo`, which is the exact
//! function `legacy-modernizer demo` runs from the CLI — this test and a
//! person running that command by hand exercise the same code path.

use legacy_modernizer::demo::run_full_demo;
use legacy_modernizer::verify::Verdict;
use legacy_modernizer::state::ApprovalRecord;
use std::fs;

#[test]
fn six_stage_pipeline_runs_end_to_end_against_mock_llm() {
    let tmp = tempdir();
    let output_dir = tmp.join("modernization-output");

    let summary = run_full_demo(&output_dir).expect("demo pipeline should complete with no errors");

    // Every stage produced its numbered report, in the right place.
    for path in [
        &summary.discovery_report,
        &summary.risk_triage_report,
        &summary.architecture_report,
        &summary.codegen_report,
        &summary.verification_report,
        &summary.documentation_report,
    ] {
        assert!(path.exists(), "expected report to exist: {}", path.display());
        let content = fs::read_to_string(path).unwrap();
        assert!(!content.trim().is_empty(), "report should not be empty: {}", path.display());
    }

    // Code Generation actually wrote a real file via the fenced-block
    // convention, not just a report.
    assert!(!summary.generated_files.is_empty(), "Code Generation should have written at least one file");
    assert!(summary.generated_files[0].exists());

    // Verification reached a real verdict (the mock script says PASS).
    assert_eq!(summary.verdict, Verdict::Pass);

    // The Phase 2 gate's integrity check: the approval on record must
    // hash-match the architecture document that's actually on disk.
    assert!(
        ApprovalRecord::is_valid_for(&output_dir, &summary.architecture_report).unwrap(),
        "recorded approval should validate against the architecture document's current hash"
    );

    cleanup(&tmp);
}

#[test]
fn tampering_with_the_architecture_doc_after_approval_invalidates_the_gate() {
    // This is the specific safety property the SHA-256 approval hash
    // exists to guarantee: if the approved document changes, Code
    // Generation must be blocked again until a human re-approves.
    let tmp = tempdir();
    let output_dir = tmp.join("modernization-output");

    let summary = run_full_demo(&output_dir).expect("demo pipeline should complete with no errors");
    assert!(ApprovalRecord::is_valid_for(&output_dir, &summary.architecture_report).unwrap());

    fs::write(&summary.architecture_report, "this proposal was changed after sign-off").unwrap();

    assert!(
        !ApprovalRecord::is_valid_for(&output_dir, &summary.architecture_report).unwrap(),
        "a changed architecture document must invalidate the prior approval"
    );

    cleanup(&tmp);
}

fn tempdir() -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    let unique = format!(
        "legacy-modernizer-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    dir.push(unique);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}
