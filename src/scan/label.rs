//! Source line classification for configurable content filtering.

#[cfg(any(
    feature = "lang-rust",
    feature = "lang-typescript",
    feature = "lang-javascript",
    feature = "lang-python",
    feature = "lang-go"
))]
use crate::scan::language::SourceLanguage;

mod regex_fallback;

#[cfg(any(
    feature = "lang-rust",
    feature = "lang-typescript",
    feature = "lang-javascript",
    feature = "lang-python",
    feature = "lang-go"
))]
mod treesitter;

/// Classification used by the `include_*` content settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineLabel {
    Code,
    Import,
    Doc,
    Comment,
}

/// Label every source line, preserving the original line count and order.
pub fn label_lines(relative_path: &str, source: &str) -> Vec<LineLabel> {
    #[cfg(any(
        feature = "lang-rust",
        feature = "lang-typescript",
        feature = "lang-javascript",
        feature = "lang-python",
        feature = "lang-go"
    ))]
    let language = SourceLanguage::from_path(relative_path);

    #[cfg(any(
        feature = "lang-rust",
        feature = "lang-typescript",
        feature = "lang-javascript",
        feature = "lang-python",
        feature = "lang-go"
    ))]
    if language.is_tree_sitter_supported() {
        if let Some(labels) = treesitter::label_lines(language, source) {
            let mut labels = apply_docstring_labels(source, labels);
            let fallback = regex_fallback::label_lines(relative_path, source);
            for (label, fallback_label) in labels.iter_mut().zip(fallback) {
                if matches!(label, LineLabel::Code)
                    && matches!(fallback_label, LineLabel::Doc | LineLabel::Comment)
                {
                    *label = fallback_label;
                }
            }
            return labels;
        }
    }

    regex_fallback::label_lines(relative_path, source)
}

#[cfg(any(
    feature = "lang-rust",
    feature = "lang-typescript",
    feature = "lang-javascript",
    feature = "lang-python",
    feature = "lang-go"
))]
fn apply_docstring_labels(source: &str, mut labels: Vec<LineLabel>) -> Vec<LineLabel> {
    let mut delimiter: Option<&'static str> = None;
    for (index, line) in source.split('\n').enumerate() {
        let trimmed = line.trim();
        if let Some(active) = delimiter {
            labels[index] = LineLabel::Doc;
            if trimmed.contains(active) {
                delimiter = None;
            }
            continue;
        }

        let found = if trimmed.starts_with("\"\"\"") {
            Some("\"\"\"")
        } else if trimmed.starts_with("'''") {
            Some("'''")
        } else {
            None
        };
        if let Some(active) = found {
            labels[index] = LineLabel::Doc;
            if trimmed[3..].contains(active) {
                delimiter = None;
            } else {
                delimiter = Some(active);
            }
        }
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_rust_imports_and_comments() {
        let source = "use crate::thing;\n/// docs\n// note\nfn main() {}\n";
        let labels = label_lines("src/main.rs", source);
        assert_eq!(
            labels,
            vec![
                LineLabel::Import,
                LineLabel::Doc,
                LineLabel::Comment,
                LineLabel::Code,
                LineLabel::Code,
            ]
        );
    }

    #[test]
    fn inline_comment_keeps_code_label() {
        let labels = label_lines("main.py", "value = 1  # comment\n");
        assert_eq!(labels[0], LineLabel::Code);
    }

    #[test]
    fn unknown_language_uses_fallback() {
        let labels = label_lines("notes.txt", "# comment\nbody\n");
        assert_eq!(labels[0], LineLabel::Comment);
        assert_eq!(labels[1], LineLabel::Code);
    }
}
