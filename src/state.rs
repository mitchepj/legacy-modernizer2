//! Run ledger, Phase -1 qualification record, and Phase 2 approval
//! record — all persisted as JSON under `<output_dir>/.legacy-modernizer/`
//! so that gates enforced in one CLI invocation are still enforced in the
//! next one (e.g. `generate` run tomorrow must still see today's approval).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

fn state_dir(output_dir: &Path) -> PathBuf {
    output_dir.join(".legacy-modernizer")
}

fn ensure_state_dir(output_dir: &Path) -> Result<PathBuf> {
    let dir = state_dir(output_dir);
    fs::create_dir_all(&dir)
        .with_context(|| format!("could not create state directory: {}", dir.display()))?;
    Ok(dir)
}

// ---------------------------------------------------------------------
// Phase -1: Model & Tooling Qualification
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualificationRecord {
    pub model: String,
    pub base_url: String,
    pub measured_context_ceiling_words: usize,
    pub qualified_at: String,
    pub notes: String,
}

impl QualificationRecord {
    pub fn save(&self, output_dir: &Path) -> Result<()> {
        let dir = ensure_state_dir(output_dir)?;
        let path = dir.join("qualification.json");
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&path, json).with_context(|| format!("could not write {}", path.display()))?;
        Ok(())
    }

    pub fn load(output_dir: &Path) -> Result<Option<Self>> {
        let path = state_dir(output_dir).join("qualification.json");
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)?;
        Ok(Some(serde_json::from_str(&raw)?))
    }
}

// ---------------------------------------------------------------------
// Phase 2: Architecture -> Code Generation human approval gate
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub approver: String,
    pub approved_at: String,
    /// SHA-256 of the approved architecture document's bytes, at the
    /// moment of approval. `generate` refuses to run if the current
    /// architecture document's hash no longer matches this — that's the
    /// integrity guard against "the document changed after sign-off."
    pub architecture_doc_sha256: String,
    pub notes: String,
}

impl ApprovalRecord {
    pub fn save(&self, output_dir: &Path) -> Result<()> {
        let dir = ensure_state_dir(output_dir)?;
        let path = dir.join("approvals.json");
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&path, json).with_context(|| format!("could not write {}", path.display()))?;
        Ok(())
    }

    pub fn load(output_dir: &Path) -> Result<Option<Self>> {
        let path = state_dir(output_dir).join("approvals.json");
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)?;
        Ok(Some(serde_json::from_str(&raw)?))
    }

    /// True only if an approval exists AND its recorded hash still
    /// matches the architecture document on disk right now.
    pub fn is_valid_for(output_dir: &Path, architecture_doc_path: &Path) -> Result<bool> {
        let record = match Self::load(output_dir)? {
            Some(r) => r,
            None => return Ok(false),
        };
        if !architecture_doc_path.exists() {
            return Ok(false);
        }
        let current_hash = hash_file(architecture_doc_path)?;
        Ok(current_hash == record.architecture_doc_sha256)
    }
}

pub fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    Ok(hash_bytes(&bytes))
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------
// Run ledger — a simple append-only log of which stages have run
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub stage: String,
    pub started_at: String,
    pub finished_at: String,
    pub status: String, // "ok" | "failed"
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunLedger {
    pub entries: Vec<LedgerEntry>,
}

impl RunLedger {
    fn path(output_dir: &Path) -> PathBuf {
        state_dir(output_dir).join("ledger.json")
    }

    pub fn load(output_dir: &Path) -> Result<Self> {
        let path = Self::path(output_dir);
        if !path.exists() {
            return Ok(RunLedger::default());
        }
        let raw = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn append(output_dir: &Path, entry: LedgerEntry) -> Result<()> {
        ensure_state_dir(output_dir)?;
        let mut ledger = Self::load(output_dir)?;
        ledger.entries.push(entry);
        let json = serde_json::to_string_pretty(&ledger)?;
        fs::write(Self::path(output_dir), json)?;
        Ok(())
    }
}

pub fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}
