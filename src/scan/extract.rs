//! Normalize source text and split into chunks.

use sha2::{Digest, Sha256};

use crate::domain::content::Chunk;

/// Extraction / normalization options (MVP defaults from product requirements).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractOptions {
    pub min_lines: usize,
    pub max_lines: usize,
    pub tab_width: usize,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            min_lines: 5,
            max_lines: 40,
            tab_width: 4,
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

/// Hash normalized chunk body (SHA-256 hex).
pub fn hash_normalized(normalized: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Extract chunks from source file text.
///
/// Display line ranges use original 1-based line numbers of kept lines.
pub fn extract_chunks(relative_path: &str, original: &str, opts: ExtractOptions) -> Vec<Chunk> {
    let line_map = map_original_to_normalized_lines(original, opts.tab_width);
    if line_map.is_empty() {
        return Vec::new();
    }

    let normalized_lines: Vec<&str> = line_map.iter().map(|(_, n)| n.as_str()).collect();
    let blocks = split_into_chunks(&normalized_lines, opts.min_lines, opts.max_lines);

    let mut chunks = Vec::new();
    for block in blocks {
        if block.is_empty() {
            continue;
        }
        let body = block
            .iter()
            .map(|&i| normalized_lines[i])
            .collect::<Vec<_>>()
            .join("\n");
        if body.trim().is_empty() {
            continue;
        }
        let first = block[0];
        let last = *block.last().unwrap();
        chunks.push(Chunk {
            relative_path: relative_path.to_string(),
            start_line: line_map[first].0,
            end_line: line_map[last].0,
            hash: hash_normalized(&body),
            normalized: body,
        });
    }
    chunks
}

fn split_into_chunks(lines: &[&str], min_lines: usize, max_lines: usize) -> Vec<Vec<usize>> {
    let min_lines = min_lines.max(1);
    let max_lines = max_lines.max(min_lines);

    let mut raw_blocks: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.is_empty() {
            if !current.is_empty() {
                raw_blocks.push(std::mem::take(&mut current));
            }
        } else {
            current.push(i);
        }
    }
    if !current.is_empty() {
        raw_blocks.push(current);
    }

    let mut sized: Vec<Vec<usize>> = Vec::new();
    for block in raw_blocks {
        if block.len() <= max_lines {
            sized.push(block);
        } else {
            for piece in block.chunks(max_lines) {
                sized.push(piece.to_vec());
            }
        }
    }

    // Absorb undersized blocks into the previous chunk when possible.
    let mut merged: Vec<Vec<usize>> = Vec::new();
    for block in sized {
        if block.len() < min_lines {
            if let Some(prev) = merged.last_mut() {
                prev.extend(block);
                continue;
            }
        }
        merged.push(block);
    }

    // Absorb a trailing undersized block into the previous one.
    if merged.len() >= 2 {
        if let Some(last_len) = merged.last().map(Vec::len) {
            if last_len < min_lines {
                let last = merged.pop().unwrap();
                merged.last_mut().unwrap().extend(last);
            }
        }
    }

    // Single undersized file: keep as one chunk so tiny fixtures remain typeable.
    merged
}

fn map_original_to_normalized_lines(original: &str, tab_width: usize) -> Vec<(u32, String)> {
    let width = tab_width.max(1);
    let mut out = Vec::new();
    for (idx, line) in original.split('\n').enumerate() {
        let line_no = (idx + 1) as u32;
        if !line.is_ascii() {
            continue;
        }
        let expanded = expand_tabs(line, width);
        out.push((line_no, expanded.trim_end().to_string()));
    }
    out
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
    fn splits_on_blank_lines_with_min() {
        let mut src = String::new();
        for i in 0..5 {
            src.push_str(&format!("a{i}\n"));
        }
        src.push('\n');
        for i in 0..5 {
            src.push_str(&format!("b{i}\n"));
        }
        let chunks = extract_chunks("f.rs", &src, ExtractOptions::default());
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn absorbs_small_block() {
        let mut src = String::new();
        for i in 0..5 {
            src.push_str(&format!("a{i}\n"));
        }
        src.push('\n');
        src.push_str("tiny\n");
        let chunks = extract_chunks("f.rs", &src, ExtractOptions::default());
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].normalized.contains("tiny"));
    }

    #[test]
    fn mechanical_split_over_max() {
        let mut src = String::new();
        for i in 0..90 {
            src.push_str(&format!("line{i}\n"));
        }
        let chunks = extract_chunks(
            "f.rs",
            &src,
            ExtractOptions {
                min_lines: 5,
                max_lines: 40,
                tab_width: 4,
            },
        );
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(c.normalized.lines().count() <= 40);
        }
    }

    #[test]
    fn tiny_file_still_one_chunk() {
        let chunks = extract_chunks("f.rs", "a\nb\n", ExtractOptions::default());
        assert_eq!(chunks.len(), 1);
    }
}
