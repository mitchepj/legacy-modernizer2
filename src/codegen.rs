//! Stage 4 — Code Generation Agent.
//!
//! Gated: the CLI (`main.rs`) must confirm a valid, hash-matching
//! approval record exists before calling `run`. This module itself
//! doesn't re-check the gate — that's deliberate separation of concerns
//! (state.rs owns gate validity; main.rs enforces it at the call site),
//! but it means `run` must never be wired to a subcommand that skips the
//! check.

use crate::codeblocks::extract_file_blocks;
use crate::llm::LlmClient;
use crate::prompts::{CODE_GENERATION_AGENT, SAFETY_PREAMBLE};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub const REPORT_FILENAME: &str = "04-codegen.md";

pub struct CodegenResult {
    pub report_path: PathBuf,
    pub files_written: Vec<PathBuf>,
}

pub fn run(
    llm: &dyn LlmClient,
    architecture_doc: &str,
    module_context: &str,
    output_dir: &Path,
) -> Result<CodegenResult> {
    let system_prompt = format!("{SAFETY_PREAMBLE}\n{CODE_GENERATION_AGENT}");
    let user_prompt = format!(
        "APPROVED ARCHITECTURE PROPOSAL:\n{architecture_doc}\n\nMODULE(S) TO TRANSLATE (from Discovery, plus raw legacy source for this batch):\n{module_context}\n"
    );

    let response = llm
        .chat(&system_prompt, &user_prompt)
        .context("Code Generation Agent call failed")?;

    let report_path = output_dir.join(REPORT_FILENAME);
    fs::create_dir_all(output_dir)?;
    fs::write(&report_path, &response)
        .with_context(|| format!("could not write {}", report_path.display()))?;

    let generated_dir = output_dir.join("generated");
    fs::create_dir_all(&generated_dir)?;

    let blocks = extract_file_blocks(&response);
    let mut files_written = Vec::new();
    for block in blocks {
        let dest = generated_dir.join(&block.file_path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&dest, &block.content)
            .with_context(|| format!("could not write generated file {}", dest.display()))?;
        files_written.push(dest);
    }

    Ok(CodegenResult { report_path, files_written })
}
