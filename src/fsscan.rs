//! Legacy-source directory walking and context-ceiling-aware batching.
//!
//! The Discovery Agent is the only agent that ever reads raw source
//! (Section 0 of the master prompt). Everything downstream consumes its
//! structured output instead. This module is the thing that actually
//! walks the directory and hands Discovery batches small enough to fit
//! the qualified model's context ceiling.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Directory names never walked into, regardless of engagement config.
/// These are near-universally build output, VCS metadata, or vendored
/// dependency trees — never original legacy source a human wrote.
const DEFAULT_EXCLUDED_DIRS: &[&str] = &[
    ".git", ".svn", ".hg", "node_modules", "target", "build", "dist",
    "out", ".idea", ".vscode", "vendor", "__pycache__", ".cargo",
];

/// Extensions skipped outright — binaries, images, archives. If Discovery
/// can't read it as text, it isn't source.
const SKIPPED_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "pdf", "zip", "tar", "gz",
    "bz2", "7z", "exe", "dll", "so", "dylib", "o", "obj", "a", "lib",
    "class", "jar", "bin", "dat",
];

#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub content: String,
}

/// Walks `root`, skipping excluded directories and non-text extensions,
/// and returns every remaining file's path + text content. Files that
/// fail to decode as UTF-8 are skipped with a note (never crash the
/// walk over one binary file the extension list didn't catch).
pub fn gather_source_files(root: &Path, extra_excluded_dirs: &[String]) -> Result<(Vec<SourceFile>, Vec<String>)> {
    let mut files = Vec::new();
    let mut skipped = Vec::new();

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            if !e.file_type().is_dir() {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            if DEFAULT_EXCLUDED_DIRS.contains(&name.as_ref()) {
                return false;
            }
            !extra_excluded_dirs.iter().any(|d| d == name.as_ref())
        })
    {
        let entry = entry.with_context(|| format!("error walking {}", root.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path().to_path_buf();

        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if SKIPPED_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
                continue;
            }
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => files.push(SourceFile { path, content }),
            Err(_) => skipped.push(format!("{} (not valid UTF-8 text — skipped)", path.display())),
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((files, skipped))
}

/// Splits `files` into batches whose combined word count stays under
/// `max_words`. A single file larger than `max_words` on its own still
/// gets its own (oversized) batch rather than being silently truncated —
/// truncating source text is exactly the kind of silent information loss
/// the Discovery Agent's prompt explicitly warns against.
pub fn batch_by_word_count(files: &[SourceFile], max_words: usize) -> Vec<Vec<&SourceFile>> {
    let mut batches: Vec<Vec<&SourceFile>> = Vec::new();
    let mut current: Vec<&SourceFile> = Vec::new();
    let mut current_words = 0usize;

    for f in files {
        let words = f.content.split_whitespace().count();
        if !current.is_empty() && current_words + words > max_words {
            batches.push(std::mem::take(&mut current));
            current_words = 0;
        }
        current.push(f);
        current_words += words;
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

/// Renders a batch of files into the flat "here is the raw source"
/// text block every Discovery prompt call includes as its user message.
pub fn render_batch(batch: &[&SourceFile], root: &Path) -> String {
    let mut out = String::new();
    for f in batch {
        let rel = f.path.strip_prefix(root).unwrap_or(&f.path);
        out.push_str(&format!("--- FILE: {} ---\n", rel.display()));
        out.push_str(&f.content);
        out.push_str("\n\n");
    }
    out
}
