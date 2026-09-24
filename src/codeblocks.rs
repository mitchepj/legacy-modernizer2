//! Parser for the fenced-code-block-with-filename convention that the
//! Code Generation and Verification agent prompts use to emit files: a
//! fenced block whose info string is `<language> file=<relative/path>`,
//! e.g. a Rust block annotated `rust file=pacemaker_core/src/battery.rs`.
//!
//! This is the one parseable contract between an LLM's free-text Markdown
//! response and this tool's file-writing logic, so it's covered by unit
//! tests (below) with no LLM involved -- pure string parsing.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlock {
    pub language: String,
    pub file_path: String,
    pub content: String,
}

/// Scans `markdown` for fenced code blocks whose info string matches
/// `<language> file=<path>`, and returns each one found, in order.
/// Fenced blocks that don't carry a `file=` annotation (e.g. an inline
/// example inside prose) are intentionally ignored — only annotated
/// blocks are meant to be written to disk.
pub fn extract_file_blocks(markdown: &str) -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    let mut lines = markdown.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("```") {
            continue;
        }
        let info = trimmed.trim_start_matches('`').trim();
        let Some(file_path) = parse_file_annotation(info) else {
            // Not an annotated block — skip to its closing fence (if any)
            // and continue scanning; this block is not written to disk.
            for skip_line in lines.by_ref() {
                if skip_line.trim_start().starts_with("```") {
                    break;
                }
            }
            continue;
        };
        let language = info.split_whitespace().next().unwrap_or("").to_string();

        let mut content_lines = Vec::new();
        for body_line in lines.by_ref() {
            if body_line.trim_start().starts_with("```") {
                break;
            }
            content_lines.push(body_line);
        }

        blocks.push(CodeBlock {
            language,
            file_path,
            content: content_lines.join("\n"),
        });
    }

    blocks
}

/// Parses an info-string like `rust file=pacemaker_core/src/battery.rs`
/// and returns the path portion, or `None` if the string carries no
/// `file=` annotation at all.
fn parse_file_annotation(info: &str) -> Option<String> {
    info.split_whitespace()
        .find_map(|token| token.strip_prefix("file="))
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_single_annotated_block() {
        let md = "Some prose.\n\n```rust file=src/lib.rs\nfn main() {}\n```\n\nMore prose.";
        let blocks = extract_file_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].language, "rust");
        assert_eq!(blocks[0].file_path, "src/lib.rs");
        assert_eq!(blocks[0].content, "fn main() {}");
    }

    #[test]
    fn extracts_multiple_annotated_blocks_in_order() {
        let md = "```c file=a.c\nint a();\n```\n\ntext between\n\n```c file=b.c\nint b();\n```\n";
        let blocks = extract_file_blocks(md);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].file_path, "a.c");
        assert_eq!(blocks[1].file_path, "b.c");
    }

    #[test]
    fn ignores_blocks_without_file_annotation() {
        let md = "```rust\n// just an example, not meant to be written\nfn x() {}\n```\n\n```python file=out.py\nprint(1)\n```\n";
        let blocks = extract_file_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].file_path, "out.py");
    }

    #[test]
    fn empty_markdown_yields_no_blocks() {
        assert!(extract_file_blocks("").is_empty());
    }

    #[test]
    fn preserves_multiline_content_exactly() {
        let md = "```rust file=src/x.rs\nfn f() {\n    let x = 1;\n    x + 1;\n}\n```\n";
        let blocks = extract_file_blocks(md);
        assert_eq!(blocks[0].content, "fn f() {\n    let x = 1;\n    x + 1;\n}");
    }
}
