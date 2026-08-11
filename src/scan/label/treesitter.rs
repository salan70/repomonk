//! Tree-sitter based line labeling for the first supported language tier.

use tree_sitter::{Language, Node, Parser};

use crate::scan::language::SourceLanguage;

use super::LineLabel;

#[derive(Debug, Clone, Copy)]
enum MarkKind {
    Import,
    Doc,
    Comment,
}

#[derive(Debug, Clone, Copy)]
struct Mark {
    start_byte: usize,
    end_byte: usize,
    kind: MarkKind,
}

pub(super) fn label_lines(language: SourceLanguage, source: &str) -> Option<Vec<LineLabel>> {
    let language = parser_language(language)?;
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;
    if tree.root_node().has_error() {
        return None;
    }

    let mut marks = Vec::new();
    collect_marks(tree.root_node(), source, &mut marks);
    Some(classify_lines(source, &marks))
}

fn parser_language(language: SourceLanguage) -> Option<Language> {
    match language {
        SourceLanguage::Rust => {
            #[cfg(feature = "lang-rust")]
            {
                Some(tree_sitter_rust::LANGUAGE.into())
            }
            #[cfg(not(feature = "lang-rust"))]
            {
                None
            }
        }
        SourceLanguage::TypeScript => {
            #[cfg(feature = "lang-typescript")]
            {
                Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            }
            #[cfg(not(feature = "lang-typescript"))]
            {
                None
            }
        }
        SourceLanguage::Tsx => {
            #[cfg(feature = "lang-typescript")]
            {
                Some(tree_sitter_typescript::LANGUAGE_TSX.into())
            }
            #[cfg(not(feature = "lang-typescript"))]
            {
                None
            }
        }
        SourceLanguage::JavaScript | SourceLanguage::Jsx => {
            #[cfg(feature = "lang-javascript")]
            {
                Some(tree_sitter_javascript::LANGUAGE.into())
            }
            #[cfg(not(feature = "lang-javascript"))]
            {
                None
            }
        }
        SourceLanguage::Python => {
            #[cfg(feature = "lang-python")]
            {
                Some(tree_sitter_python::LANGUAGE.into())
            }
            #[cfg(not(feature = "lang-python"))]
            {
                None
            }
        }
        SourceLanguage::Go => {
            #[cfg(feature = "lang-go")]
            {
                Some(tree_sitter_go::LANGUAGE.into())
            }
            #[cfg(not(feature = "lang-go"))]
            {
                None
            }
        }
        SourceLanguage::Unknown => None,
    }
}

fn collect_marks(node: Node<'_>, source: &str, marks: &mut Vec<Mark>) {
    let kind = node.kind();
    if kind == "comment" {
        let mark_kind = if is_doc_comment(source, node) {
            MarkKind::Doc
        } else {
            MarkKind::Comment
        };
        marks.push(Mark {
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            kind: mark_kind,
        });
    } else if is_import_node(kind) {
        marks.push(Mark {
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            kind: MarkKind::Import,
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_marks(child, source, marks);
    }
}

fn is_import_node(kind: &str) -> bool {
    matches!(
        kind,
        "use_declaration"
            | "extern_crate_declaration"
            | "import_statement"
            | "import_declaration"
            | "import_from_statement"
            | "import_clause"
            | "import_specifier"
    )
}

fn is_doc_comment(source: &str, node: Node<'_>) -> bool {
    source
        .get(node.start_byte()..node.end_byte())
        .map(|text| {
            let trimmed = text.trim_start();
            trimmed.starts_with("///")
                || trimmed.starts_with("//!")
                || trimmed.starts_with("/**")
                || trimmed.starts_with("##")
        })
        .unwrap_or(false)
}

fn classify_lines(source: &str, marks: &[Mark]) -> Vec<LineLabel> {
    let line_starts = line_starts(source);
    line_starts
        .iter()
        .enumerate()
        .map(|(index, &start)| {
            let end = line_starts
                .get(index + 1)
                .copied()
                .unwrap_or(source.len())
                .saturating_sub(usize::from(index + 1 < line_starts.len()));
            let line_marks: Vec<Mark> = marks
                .iter()
                .copied()
                .filter(|mark| mark.end_byte > start && mark.start_byte < end)
                .collect();

            let has_code = (start..end).any(|byte| {
                let value = source.as_bytes()[byte];
                !value.is_ascii_whitespace()
                    && !line_marks
                        .iter()
                        .any(|mark| byte >= mark.start_byte && byte < mark.end_byte)
            });
            if has_code {
                return LineLabel::Code;
            }
            if line_marks
                .iter()
                .any(|mark| matches!(mark.kind, MarkKind::Import))
            {
                LineLabel::Import
            } else if line_marks
                .iter()
                .any(|mark| matches!(mark.kind, MarkKind::Doc))
            {
                LineLabel::Doc
            } else if line_marks
                .iter()
                .any(|mark| matches!(mark.kind, MarkKind::Comment))
            {
                LineLabel::Comment
            } else {
                LineLabel::Code
            }
        })
        .collect()
}

fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}
