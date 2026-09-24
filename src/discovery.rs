//! Stage 1 — Discovery & Inventory Agent.
//!
//! The only stage that reads raw source. Walks the legacy directory,
//! batches files under the qualified context ceiling, and asks the model
//! to build a structured (JSON) semantic index that every later stage
//! consumes instead of re-reading raw files.

use crate::fsscan::{batch_by_word_count, gather_source_files, render_batch};
use crate::llm::LlmClient;
use crate::prompts::{DISCOVERY_AGENT, SAFETY_PREAMBLE};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const REPORT_FILENAME: &str = "01-discovery.json";

pub fn run(
    llm: &dyn LlmClient,
    source_dir: &Path,
    output_dir: &Path,
    extra_excluded_dirs: &[String],
    max_words: usize,
) -> Result<PathBuf> {
    let (files, skipped) = gather_source_files(source_dir, extra_excluded_dirs)?;
    if files.is_empty() {
        anyhow::bail!(
            "no readable source files found under {} (after exclusions) — nothing for Discovery to inventory",
            source_dir.display()
        );
    }

    let batches = batch_by_word_count(&files, max_words);
    let system_prompt = format!("{SAFETY_PREAMBLE}\n{DISCOVERY_AGENT}");

    let mut all_modules = Vec::new();
    let mut all_notes: Vec<Value> = skipped.iter().map(|s| json!(s)).collect();

    for batch in &batches {
        let user_prompt = render_batch(batch, source_dir);
        let response = llm
            .chat(&system_prompt, &user_prompt)
            .context("Discovery Agent call failed")?;
        let parsed: Value = parse_json_response(&response)
            .context("Discovery Agent response was not valid JSON matching the required schema")?;

        if let Some(modules) = parsed.get("modules").and_then(|m| m.as_array()) {
            all_modules.extend(modules.iter().cloned());
        }
        if let Some(notes) = parsed.get("notes_for_human_review").and_then(|n| n.as_array()) {
            all_notes.extend(notes.iter().cloned());
        }
    }

    let combined = json!({
        "modules": all_modules,
        "notes_for_human_review": all_notes,
        "batches_processed": batches.len(),
        "files_inventoried": files.len(),
    });

    let output_path = output_dir.join(REPORT_FILENAME);
    std::fs::create_dir_all(output_dir)?;
    std::fs::write(&output_path, serde_json::to_string_pretty(&combined)?)
        .with_context(|| format!("could not write {}", output_path.display()))?;

    Ok(output_path)
}

/// Model responses are supposed to be pure JSON, but real models
/// sometimes wrap it in a fenced code block or add a stray sentence.
/// Tolerate that by extracting the first `{...}` span before parsing.
pub fn parse_json_response(response: &str) -> Result<Value> {
    let trimmed = response.trim();
    let candidate = if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        &trimmed[start..=end]
    } else {
        trimmed
    };
    serde_json::from_str(candidate).context("could not parse model response as JSON")
}
