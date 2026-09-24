//! Stage 5 — Verification Agent. A hard gate: produces a generated test
//! suite plus a behavioral/static-analysis review and a PASS / FAIL /
//! FAIL-WITH-CONDITIONS verdict. Never lowers the bar to pass a module.

use crate::codeblocks::extract_file_blocks;
use crate::llm::LlmClient;
use crate::prompts::{SAFETY_PREAMBLE, VERIFICATION_AGENT};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub const REPORT_FILENAME: &str = "05-verification.md";

pub struct VerificationResult {
    pub report_path: PathBuf,
    pub test_files_written: Vec<PathBuf>,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    FailWithConditions,
    Fail,
    Unknown,
}

pub fn run(
    llm: &dyn LlmClient,
    legacy_module: &str,
    migrated_code: &str,
    architecture_doc: &str,
    output_dir: &Path,
) -> Result<VerificationResult> {
    let system_prompt = format!("{SAFETY_PREAMBLE}\n{VERIFICATION_AGENT}");
    let user_prompt = format!(
        "LEGACY MODULE:\n{legacy_module}\n\nMIGRATED CODE:\n{migrated_code}\n\nARCHITECTURE PROPOSAL:\n{architecture_doc}\n"
    );

    let response = llm
        .chat(&system_prompt, &user_prompt)
        .context("Verification Agent call failed")?;

    let report_path = output_dir.join(REPORT_FILENAME);
    fs::create_dir_all(output_dir)?;
    fs::write(&report_path, &response)
        .with_context(|| format!("could not write {}", report_path.display()))?;

    let generated_dir = output_dir.join("generated");
    fs::create_dir_all(&generated_dir)?;
    let mut test_files_written = Vec::new();
    for block in extract_file_blocks(&response) {
        let dest = generated_dir.join(&block.file_path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&dest, &block.content)
            .with_context(|| format!("could not write test file {}", dest.display()))?;
        test_files_written.push(dest);
    }

    let verdict = parse_verdict(&response);

    Ok(VerificationResult { report_path, test_files_written, verdict })
}

fn parse_verdict(response: &str) -> Verdict {
    let upper = response.to_uppercase();
    // Order matters: check the more specific phrase before the general one.
    if upper.contains("FAIL-WITH-CONDITIONS") || upper.contains("FAIL WITH CONDITIONS") {
        Verdict::FailWithConditions
    } else if upper.contains("VERDICT: FAIL") || upper.contains("VERDICT:FAIL") {
        Verdict::Fail
    } else if upper.contains("VERDICT: PASS") || upper.contains("VERDICT:PASS") {
        Verdict::Pass
    } else {
        Verdict::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pass_verdict() {
        assert_eq!(parse_verdict("## Verdict\nVerdict: PASS"), Verdict::Pass);
    }

    #[test]
    fn parses_fail_with_conditions_before_plain_fail() {
        assert_eq!(
            parse_verdict("## Verdict\nVerdict: FAIL-WITH-CONDITIONS, see above"),
            Verdict::FailWithConditions
        );
    }

    #[test]
    fn parses_fail_verdict() {
        assert_eq!(parse_verdict("Verdict: FAIL"), Verdict::Fail);
    }

    #[test]
    fn unknown_when_no_verdict_present() {
        assert_eq!(parse_verdict("no verdict line here"), Verdict::Unknown);
    }
}
