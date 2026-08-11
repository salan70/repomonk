//! Normalize source text into a single file-body typing unit.

use sha2::{Digest, Sha256};

use crate::domain::content::Chunk;
use crate::scan::label::{label_lines, LineLabel};

/// Extraction / normalization options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractOptions {
    pub tab_width: usize,
    pub include_imports: bool,
    pub include_doc_comments: bool,
    pub include_comments: bool,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            tab_width: 4,
            include_imports: false,
            include_doc_comments: true,
            include_comments: false,
        }
    }
}

/// Normalize raw file text for typing.
///
/// - Tabs → spaces (`tab_width`)
/// - Strip trailing whitespace per line
/// - Drop lines containing non-ASCII characters
pub fn normalize(text: &str, tab_width: usize) -> String {
    let width = tab_width.max(1);
    let mut out_lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        if !line.is_ascii() {
            continue;
        }
        let expanded = expand_tabs(line, width);
        let trimmed = expanded.trim_end();
        out_lines.push(trimmed.to_string());
    }
    out_lines.join("\n")
}

fn expand_tabs(line: &str, tab_width: usize) -> String {
    let mut out = String::with_capacity(line.len());
    let mut col = 0usize;
    for ch in line.chars() {
        if ch == '\t' {
            let spaces = tab_width - (col % tab_width);
            for _ in 0..spaces {
                out.push(' ');
                col += 1;
            }
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out
}

/// Hash normalized body (SHA-256 hex).
pub fn hash_normalized(normalized: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Extract a single typing unit from source file text.
///
/// Display line ranges use original 1-based line numbers of kept lines.
/// Returns an empty vec when nothing remains after normalization.
pub fn extract_chunks(relative_path: &str, original: &str, opts: ExtractOptions) -> Vec<Chunk> {
    let line_map = map_original_to_normalized_lines(relative_path, original, opts);
    if line_map.is_empty() {
        return Vec::new();
    }

    let body = line_map
        .iter()
        .map(|(_, n)| n.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if body.trim().is_empty() {
        return Vec::new();
    }

    let start_line = line_map[0].0;
    let end_line = line_map[line_map.len() - 1].0;
    vec![Chunk {
        relative_path: relative_path.to_string(),
        start_line,
        end_line,
        hash: hash_normalized(&body),
        normalized: body,
    }]
}

fn map_original_to_normalized_lines(
    relative_path: &str,
    original: &str,
    opts: ExtractOptions,
) -> Vec<(u32, String)> {
    let width = opts.tab_width.max(1);
    let labels = label_lines(relative_path, original);
    let mut out = Vec::new();
    for (idx, line) in original.split('\n').enumerate() {
        let line_no = (idx + 1) as u32;
        let label = labels.get(idx).copied().unwrap_or(LineLabel::Code);
        if !should_include(label, opts) {
            continue;
        }
        if !line.is_ascii() {
            continue;
        }
        let expanded = expand_tabs(line, width);
        out.push((line_no, expanded.trim_end().to_string()));
    }
    out
}

fn should_include(label: LineLabel, opts: ExtractOptions) -> bool {
    match label {
        LineLabel::Code => true,
        LineLabel::Import => opts.include_imports,
        LineLabel::Doc => opts.include_doc_comments,
        LineLabel::Comment => opts.include_comments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_trailing_ws_and_tabs() {
        let n = normalize("a\tb  \n", 4);
        assert_eq!(n, "a   b\n");
    }

    #[test]
    fn drops_non_ascii_lines() {
        let n = normalize("ok\n日本語\nok2", 4);
        assert_eq!(n, "ok\nok2");
    }

    #[test]
    fn hash_stable() {
        assert_eq!(hash_normalized("abc"), hash_normalized("abc"));
        assert_ne!(hash_normalized("abc"), hash_normalized("abd"));
    }

    #[test]
    fn whole_file_is_one_unit_across_blank_lines() {
        let mut src = String::new();
        for i in 0..5 {
            src.push_str(&format!("a{i}\n"));
        }
        src.push('\n');
        for i in 0..5 {
            src.push_str(&format!("b{i}\n"));
        }
        let chunks = extract_chunks("f.rs", &src, ExtractOptions::default());
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].normalized.contains("a0"));
        assert!(chunks[0].normalized.contains("b4"));
    }

    #[test]
    fn long_file_is_not_split() {
        let mut src = String::new();
        for i in 0..90 {
            src.push_str(&format!("line{i}\n"));
        }
        let chunks = extract_chunks("f.rs", &src, ExtractOptions::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].normalized.split('\n').count(), 91);
    }

    #[test]
    fn tiny_file_still_one_unit() {
        let chunks = extract_chunks("f.rs", "a\nb\n", ExtractOptions::default());
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn hash_matches_normalized_body() {
        let chunks = extract_chunks("f.rs", "fn main() {}\n", ExtractOptions::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].hash, hash_normalized(&chunks[0].normalized));
    }

    #[test]
    fn filters_imports_and_comments_before_normalizing() {
        let source = "use crate::thing;\n/// docs\n// note\nfn main() {}\n";
        let chunks = extract_chunks("f.rs", source, ExtractOptions::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].normalized, "/// docs\nfn main() {}\n");

        let chunks = extract_chunks(
            "f.rs",
            source,
            ExtractOptions {
                include_imports: true,
                include_comments: true,
                ..ExtractOptions::default()
            },
        );
        assert_eq!(chunks[0].normalized, source);
    }

    #[test]
    fn all_filtered_lines_produce_no_chunk() {
        let source = "use crate::thing;\n// note\n";
        let chunks = extract_chunks("f.rs", source, ExtractOptions::default());
        assert!(chunks.is_empty());
    }
}
