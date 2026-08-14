//! Repository-local import extraction and path resolution.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::domain::content::{FileStatus, ImportEdge, ScannedFile};
use crate::scan::language::SourceLanguage;

mod js_ts;
mod python;
mod rust;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportSpec {
    /// Module path used for resolution.
    pub raw: String,
    /// 1-based source line of the import.
    pub line: usize,
    /// Bound names introduced by the import.
    pub bindings: Vec<String>,
    /// Full import statement for UI.
    pub display: String,
}

/// Collect repository-local import edges in source order.
pub fn collect_edges(root: &Path, files: &[ScannedFile]) -> Vec<ImportEdge> {
    let typeable: HashSet<String> = files
        .iter()
        .filter(|file| file.status != FileStatus::Skipped && !file.chunks.is_empty())
        .map(|file| file.relative_path.clone())
        .collect();

    let mut by_pair: HashMap<(String, String), ImportEdge> = HashMap::new();
    let mut order: Vec<(String, String)> = Vec::new();
    for file in files {
        if !typeable.contains(&file.relative_path) {
            continue;
        }
        let path = root.join(&file.relative_path);
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        let language = SourceLanguage::from_path(&file.relative_path);
        let first_use = first_use_lines(&source);
        for spec in imports(language, &source) {
            if let Some(target) =
                resolve_import(&file.relative_path, language, &spec.raw, &typeable)
            {
                let use_line = spec
                    .bindings
                    .iter()
                    .filter_map(|name| first_use.get(name).copied())
                    .min();
                let key = (file.relative_path.clone(), target.clone());
                if let Some(existing) = by_pair.get_mut(&key) {
                    if spec.line < existing.decl_line {
                        existing.raw = spec.display.clone();
                    }
                    existing.decl_line = existing.decl_line.min(spec.line);
                    existing.first_use_line = min_opt(existing.first_use_line, use_line);
                } else {
                    order.push(key.clone());
                    by_pair.insert(
                        key,
                        ImportEdge {
                            importer: file.relative_path.clone(),
                            imported: target,
                            decl_line: spec.line,
                            first_use_line: use_line,
                            raw: spec.display,
                        },
                    );
                }
            }
        }
    }
    order
        .into_iter()
        .filter_map(|key| by_pair.remove(&key))
        .collect()
}

fn min_opt(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (None, None) => None,
    }
}

fn first_use_lines(source: &str) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for (index, line) in source.lines().enumerate() {
        if is_import_line(line) {
            continue;
        }
        for ident in identifiers(line) {
            map.entry(ident).or_insert(index + 1);
        }
    }
    map
}

fn is_import_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let trimmed = trimmed.strip_prefix("pub ").unwrap_or(trimmed);
    trimmed.starts_with("use ")
        || trimmed.starts_with("mod ")
        || trimmed.starts_with("import ")
        || trimmed.starts_with("import\t")
        || trimmed.starts_with("from ")
}

fn identifiers(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let push_ident = |current: &mut String, out: &mut Vec<String>| {
        if current.is_empty() {
            return;
        }
        let starts_ok = current
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
        if starts_ok {
            out.push(std::mem::take(current));
        } else {
            current.clear();
        }
    };
    for ch in line.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch);
        } else {
            push_ident(&mut current, &mut out);
        }
    }
    push_ident(&mut current, &mut out);
    out
}

fn imports(language: SourceLanguage, source: &str) -> Vec<ImportSpec> {
    match language {
        SourceLanguage::Rust => rust::imports(source),
        SourceLanguage::TypeScript
        | SourceLanguage::Tsx
        | SourceLanguage::JavaScript
        | SourceLanguage::Jsx => js_ts::imports(source),
        SourceLanguage::Python => python::imports(source),
        SourceLanguage::Go | SourceLanguage::Unknown => Vec::new(),
    }
}

fn resolve_import(
    importer: &str,
    language: SourceLanguage,
    raw: &str,
    typeable: &HashSet<String>,
) -> Option<String> {
    let candidates = match language {
        SourceLanguage::Rust => rust::candidates(importer, raw),
        SourceLanguage::TypeScript
        | SourceLanguage::Tsx
        | SourceLanguage::JavaScript
        | SourceLanguage::Jsx => js_ts::candidates(importer, raw),
        SourceLanguage::Python => python::candidates(importer, raw),
        SourceLanguage::Go | SourceLanguage::Unknown => Vec::new(),
    };

    candidates
        .into_iter()
        .map(normalize_path)
        .find(|candidate| typeable.contains(candidate))
}

fn normalize_path(path: PathBuf) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::RootDir | Component::Prefix(_) => return String::new(),
        }
    }
    parts.join("/")
}

fn path_candidates(base: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    candidates.push(base.to_path_buf());
    for extension in extensions {
        candidates.push(base.with_extension(extension));
    }
    for extension in extensions {
        candidates.push(base.join(format!("index.{extension}")));
        candidates.push(base.join(format!("mod.{extension}")));
        candidates.push(base.join(format!("__init__.{extension}")));
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::content::Chunk;
    use tempfile::tempdir;

    fn file(path: &str, body: &str) -> ScannedFile {
        ScannedFile {
            relative_path: path.into(),
            status: FileStatus::Todo,
            skip_reason: None,
            chunks: vec![Chunk {
                relative_path: path.into(),
                start_line: 1,
                end_line: 1,
                normalized: body.into(),
                hash: path.into(),
            }],
        }
    }

    #[test]
    fn collects_only_repository_local_edges() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/main.ts"),
            "import x from './x';\nimport fs from 'node:fs';\n",
        )
        .unwrap();
        fs::write(dir.path().join("src/x.ts"), "export const x = 1;\n").unwrap();

        let files = vec![
            file("src/main.ts", "import x from './x';"),
            file("src/x.ts", "export const x = 1;"),
        ];
        let edges = collect_edges(dir.path(), &files);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].importer, "src/main.ts");
        assert_eq!(edges[0].imported, "src/x.ts");
        assert_eq!(edges[0].decl_line, 1);
    }

    #[test]
    fn first_use_skips_import_lines() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub mod app;\npub mod unused;\nfn boot() { app::run(); }\n",
        )
        .unwrap();
        fs::write(dir.path().join("src/app.rs"), "pub fn run() {}\n").unwrap();
        fs::write(dir.path().join("src/unused.rs"), "pub fn x() {}\n").unwrap();

        let files = vec![
            file("src/lib.rs", "pub mod app;"),
            file("src/app.rs", "pub fn run() {}"),
            file("src/unused.rs", "pub fn x() {}"),
        ];
        let edges = collect_edges(dir.path(), &files);
        let app = edges.iter().find(|e| e.imported == "src/app.rs").unwrap();
        let unused = edges
            .iter()
            .find(|e| e.imported == "src/unused.rs")
            .unwrap();
        assert_eq!(app.first_use_line, Some(3));
        assert_eq!(unused.first_use_line, None);
        assert_eq!(app.decl_line, 1);
        assert_eq!(unused.decl_line, 2);
    }

    #[test]
    fn duplicate_edges_keep_earliest_lines() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/main.ts"),
            "import { a } from './a';\nconst x = a;\nimport { a as b } from './a';\nconst y = b;\n",
        )
        .unwrap();
        fs::write(dir.path().join("src/a.ts"), "export const a = 1;\n").unwrap();
        let files = vec![
            file("src/main.ts", "import { a } from './a';"),
            file("src/a.ts", "export const a = 1;"),
        ];
        let edges = collect_edges(dir.path(), &files);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].decl_line, 1);
        assert_eq!(edges[0].first_use_line, Some(2));
    }
}
