//! Lightweight Python import resolver.

use std::path::{Path, PathBuf};

use super::path_candidates;
use super::ImportSpec;

pub(super) fn imports(source: &str) -> Vec<ImportSpec> {
    let mut result = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("from ") {
            if let Some(module) = rest.split_whitespace().next() {
                if !module.is_empty() {
                    result.push(ImportSpec {
                        raw: module.to_string(),
                    });
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("import ") {
            for item in rest.split(',') {
                if let Some(module) = item.split_whitespace().next() {
                    if !module.is_empty() {
                        result.push(ImportSpec {
                            raw: module.to_string(),
                        });
                    }
                }
            }
        }
    }
    result
}

pub(super) fn candidates(importer: &str, raw: &str) -> Vec<PathBuf> {
    let importer_path = Path::new(importer);
    let current_dir = importer_path.parent().unwrap_or_else(|| Path::new(""));
    let dots = raw
        .chars()
        .take_while(|character| *character == '.')
        .count();
    let module = raw.trim_start_matches('.');
    let module_path = module.replace('.', "/");

    if dots > 0 {
        let mut base = current_dir.to_path_buf();
        for _ in 1..dots {
            base = base.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
        }
        return path_candidates(&base.join(module_path), &["py", "pyi"]);
    }

    let mut candidates = path_candidates(&current_dir.join(&module_path), &["py", "pyi"]);
    candidates.extend(path_candidates(&PathBuf::from(module_path), &["py", "pyi"]));
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_python_imports() {
        let imports = imports("from .pkg import value\nimport os, package.mod as mod\n");
        assert_eq!(
            imports.into_iter().map(|item| item.raw).collect::<Vec<_>>(),
            vec![".pkg", "os", "package.mod"]
        );
    }

    #[test]
    fn resolves_relative_python_modules() {
        let current = candidates("pkg/main.py", ".util");
        assert!(current.contains(&PathBuf::from("pkg/util.py")));
        let parent = super::candidates("pkg/sub/main.py", "..util");
        assert!(parent.contains(&PathBuf::from("pkg/util.py")));
    }
}
