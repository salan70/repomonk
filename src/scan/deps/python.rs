//! Lightweight Python import resolver.

use std::path::{Path, PathBuf};

use super::path_candidates;
use super::ImportSpec;

pub(super) fn imports(source: &str) -> Vec<ImportSpec> {
    let mut result = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        let display = trimmed.to_string();
        if let Some(rest) = trimmed.strip_prefix("from ") {
            if let Some(module) = rest.split_whitespace().next() {
                if !module.is_empty() {
                    let bindings = from_import_bindings(rest);
                    result.push(ImportSpec {
                        raw: module.to_string(),
                        line: index + 1,
                        bindings,
                        display: display.clone(),
                    });
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("import ") {
            for item in rest.split(',') {
                if let Some(module) = item.split_whitespace().next() {
                    if !module.is_empty() {
                        let binding = import_binding(item);
                        result.push(ImportSpec {
                            raw: module.to_string(),
                            line: index + 1,
                            bindings: binding.into_iter().collect(),
                            display: display.clone(),
                        });
                    }
                }
            }
        }
    }
    result
}

fn from_import_bindings(rest: &str) -> Vec<String> {
    let Some((_, imported)) = rest.split_once(" import ") else {
        return Vec::new();
    };
    imported
        .split(',')
        .filter_map(|item| {
            let item = item.trim();
            if item.is_empty() || item == "*" {
                return None;
            }
            let name = if let Some((_, alias)) = item.split_once(" as ") {
                alias.trim()
            } else {
                item.split_whitespace().next().unwrap_or("")
            };
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect()
}

fn import_binding(item: &str) -> Option<String> {
    let item = item.trim();
    if let Some((_, alias)) = item.split_once(" as ") {
        let alias = alias.trim();
        if alias.is_empty() {
            None
        } else {
            Some(alias.to_string())
        }
    } else {
        item.split_whitespace()
            .next()
            .and_then(|module| module.rsplit('.').next())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
    }
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
            imports
                .iter()
                .map(|item| item.raw.as_str())
                .collect::<Vec<_>>(),
            vec![".pkg", "os", "package.mod"]
        );
        assert_eq!(imports[0].bindings, vec!["value"]);
        assert_eq!(imports[1].bindings, vec!["os"]);
        assert_eq!(imports[2].bindings, vec!["mod"]);
        assert_eq!(imports[0].line, 1);
    }

    #[test]
    fn extracts_from_import_aliases() {
        let imports = imports("from .x import a, b as c\n");
        assert_eq!(imports[0].raw, ".x");
        assert_eq!(imports[0].bindings, vec!["a", "c"]);
    }

    #[test]
    fn resolves_relative_python_modules() {
        let current = candidates("pkg/main.py", ".util");
        assert!(current.contains(&PathBuf::from("pkg/util.py")));
        let parent = super::candidates("pkg/sub/main.py", "..util");
        assert!(parent.contains(&PathBuf::from("pkg/util.py")));
    }
}
