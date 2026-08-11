//! Repository-local import extraction and path resolution.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::domain::content::{FileStatus, ScannedFile};
use crate::scan::language::SourceLanguage;

mod js_ts;
mod python;
mod rust;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportSpec {
    pub raw: String,
}

/// Collect repository-local `(importer, imported)` edges in source order.
pub fn collect_edges(root: &Path, files: &[ScannedFile]) -> Vec<(String, String)> {
    let typeable: HashSet<String> = files
        .iter()
        .filter(|file| file.status != FileStatus::Skipped && !file.chunks.is_empty())
        .map(|file| file.relative_path.clone())
        .collect();

    let mut edges = Vec::new();
    let mut seen = HashSet::new();
    for file in files {
        if !typeable.contains(&file.relative_path) {
            continue;
        }
        let path = root.join(&file.relative_path);
        let Ok(source) = fs::read_to_string(path) else {
            continue;
        };
        let language = SourceLanguage::from_path(&file.relative_path);
        for spec in imports(language, &source) {
            if let Some(target) =
                resolve_import(&file.relative_path, language, &spec.raw, &typeable)
            {
                let edge = (file.relative_path.clone(), target);
                if seen.insert(edge.clone()) {
                    edges.push(edge);
                }
            }
        }
    }
    edges
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
            ScannedFile {
                relative_path: "src/main.ts".into(),
                status: FileStatus::Todo,
                skip_reason: None,
                chunks: vec![Chunk {
                    relative_path: "src/main.ts".into(),
                    start_line: 1,
                    end_line: 1,
                    normalized: "import x from './x';".into(),
                    hash: "main".into(),
                }],
            },
            ScannedFile {
                relative_path: "src/x.ts".into(),
                status: FileStatus::Todo,
                skip_reason: None,
                chunks: vec![Chunk {
                    relative_path: "src/x.ts".into(),
                    start_line: 1,
                    end_line: 1,
                    normalized: "export const x = 1;".into(),
                    hash: "x".into(),
                }],
            },
        ];
        assert_eq!(
            collect_edges(dir.path(), &files),
            vec![("src/main.ts".into(), "src/x.ts".into())]
        );
    }
}
