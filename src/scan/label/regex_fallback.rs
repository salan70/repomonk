//! Conservative line-oriented fallback for languages without a parser.

use super::LineLabel;

pub(super) fn label_lines(_relative_path: &str, source: &str) -> Vec<LineLabel> {
    let mut labels = Vec::new();
    let mut block_comment: Option<bool> = None;
    let mut triple_string: Option<&'static str> = None;

    for line in source.split('\n') {
        let trimmed = line.trim_start();

        if let Some(delimiter) = triple_string {
            labels.push(LineLabel::Doc);
            if trimmed.contains(delimiter) {
                triple_string = None;
            }
            continue;
        }

        if let Some(is_doc) = block_comment {
            labels.push(if is_doc {
                LineLabel::Doc
            } else {
                LineLabel::Comment
            });
            if trimmed.contains("*/") {
                block_comment = None;
            }
            continue;
        }

        if let Some(delimiter) = triple_delimiter(trimmed) {
            labels.push(LineLabel::Doc);
            if trimmed[3..].contains(delimiter) {
                triple_string = None;
            } else {
                triple_string = Some(delimiter);
            }
            continue;
        }

        if let Some(is_doc) = block_comment_start(trimmed) {
            labels.push(if is_doc {
                LineLabel::Doc
            } else {
                LineLabel::Comment
            });
            if !trimmed.contains("*/") {
                block_comment = Some(is_doc);
            }
            continue;
        }

        if is_import_line(trimmed) {
            labels.push(LineLabel::Import);
        } else if is_line_comment(trimmed) {
            labels.push(if is_doc_comment(trimmed) {
                LineLabel::Doc
            } else {
                LineLabel::Comment
            });
        } else {
            labels.push(LineLabel::Code);
        }
    }

    labels
}

fn is_import_line(line: &str) -> bool {
    line.starts_with("use ")
        || line.starts_with("use\t")
        || line.starts_with("extern crate ")
        || line.starts_with("import ")
        || line.starts_with("import\t")
        || line.starts_with("from ")
        || line.starts_with("require(")
        || line.starts_with("#include ")
        || line.starts_with("# include ")
        || line.starts_with("include ")
        || line.starts_with("source ")
}

fn is_line_comment(line: &str) -> bool {
    line.starts_with("//")
        || line.starts_with('#') && !line.starts_with("#!")
        || line.starts_with("--")
}

fn is_doc_comment(line: &str) -> bool {
    line.starts_with("///")
        || line.starts_with("//!")
        || line.starts_with("/**")
        || line.starts_with("##")
}

fn block_comment_start(line: &str) -> Option<bool> {
    if line.starts_with("/**") {
        Some(true)
    } else if line.starts_with("/*") {
        Some(false)
    } else {
        None
    }
}

fn triple_delimiter(line: &str) -> Option<&'static str> {
    if line.starts_with("\"\"\"") {
        Some("\"\"\"")
    } else if line.starts_with("'''") {
        Some("'''")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_multiline_comments_and_imports() {
        let source = "/** docs\n * more\n */\nimport x from \"x\";\nbody();\n";
        assert_eq!(
            label_lines("unknown.txt", source),
            vec![
                LineLabel::Doc,
                LineLabel::Doc,
                LineLabel::Doc,
                LineLabel::Import,
                LineLabel::Code,
                LineLabel::Code,
            ]
        );
    }
}
