//! `legacy-modernizer.toml` schema and loading.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Base URL of an OpenAI-compatible server, e.g.
    /// "http://127.0.0.1:8000/v1" for vLLM, or "http://127.0.0.1:11434/v1"
    /// for Ollama's OpenAI-compat endpoint. No trailing "/chat/completions".
    pub base_url: String,
    /// Optional bearer token. Most self-hosted, air-gapped servers don't
    /// require one; leave unset (or empty string) in that case.
    #[serde(default)]
    pub api_key: Option<String>,
    pub model: String,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Empirically-measured usable context ceiling for the qualified
    /// model, in words (not tokens — word count is a conservative,
    /// tokenizer-agnostic proxy). Section 7 of the master prompt requires
    /// this be re-measured against the actual self-hosted model during
    /// Phase -1 qualification, not assumed. `qualify` writes this back
    /// after measuring; the config value is the starting hypothesis.
    #[serde(default = "default_context_ceiling_words")]
    pub context_ceiling_words: usize,
}

fn default_temperature() -> f32 {
    0.2
}

fn default_context_ceiling_words() -> usize {
    2000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementConfig {
    pub name: String,
    /// Directory the pipeline's numbered reports and artifacts are
    /// written to. Never the legacy source directory itself.
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
    /// Named compliance regime to map audit-trail evidence against in
    /// the Documentation & Audit stage (e.g. "NIST 800-53"). Optional —
    /// omit for engagements with no compliance mapping requirement.
    #[serde(default)]
    pub compliance_regime: Option<String>,
    /// Directory names excluded from Discovery's filesystem walk, on top
    /// of the tool's built-in defaults (.git, node_modules, target, etc).
    #[serde(default)]
    pub extra_excluded_dirs: Vec<String>,
}

fn default_output_dir() -> String {
    "modernization-output".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApproversConfig {
    /// The Phase 2 (Architecture -> Code Generation) approver of record
    /// for this engagement. The interactive gate prompt still asks for a
    /// name at approval time and records exactly what was typed; this
    /// field is only a default suggestion shown at the prompt.
    #[serde(default)]
    pub phase2_default_approver: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub llm: LlmConfig,
    pub engagement: EngagementConfig,
    #[serde(default)]
    pub approvers: ApproversConfig,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("could not read config file: {}", path.display()))?;
        let cfg: Config = toml::from_str(&raw)
            .with_context(|| format!("could not parse config file as TOML: {}", path.display()))?;
        Ok(cfg)
    }

    /// A conservative default config, used only by `legacy-modernizer init`
    /// to scaffold a starting `legacy-modernizer.toml` for the user to edit.
    pub fn scaffold(engagement_name: &str) -> Self {
        Config {
            llm: LlmConfig {
                base_url: "http://127.0.0.1:8000/v1".to_string(),
                api_key: None,
                model: "REPLACE_ME_with_your_served_model_name".to_string(),
                temperature: default_temperature(),
                context_ceiling_words: default_context_ceiling_words(),
            },
            engagement: EngagementConfig {
                name: engagement_name.to_string(),
                output_dir: default_output_dir(),
                compliance_regime: None,
                extra_excluded_dirs: vec![],
            },
            approvers: ApproversConfig::default(),
        }
    }
}
