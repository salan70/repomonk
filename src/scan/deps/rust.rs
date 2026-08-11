//! Lightweight Rust module/import resolver.

use std::path::{Path, PathBuf};

use super::path_candidates;
use super::ImportSpec;

pub(super) fn imports(source: &str) -> Vec<ImportSpec> {
    let mut result = Vec::new();
    for line in source.lines() {
        let line = line.trim();
        let line = line.strip_prefix("pub ").unwrap_or(line);
        if let Some(rest) = line.strip_prefix("use ") {
            let path = rest
                .split(';')
                .next()
                .unwrap_or(rest)
                .split('{')
                .next()
                .unwrap_or(rest)
                .trim()
                .trim_end_matches("::");
            if !path.is_empty() {
                result.push(ImportSpec {
                    raw: path.to_string(),
                });
            }
        } else if let Some(rest) = line.strip_prefix("mod ") {
            let name = rest
                .split(';')
                .next()
                .unwrap_or(rest)
                .split_whitespace()
                .next()
                .unwrap_or_default();
            if !name.is_empty() {
                result.push(ImportSpec {
                    raw: format!("self::{name}"),
                });
            }
        }
    }
    result
}

pub(super) fn candidates(importer: &str, raw: &str) -> Vec<PathBuf> {
    let importer_path = Path::new(importer);
    let current_dir = importer_path.parent().unwrap_or_else(|| Path::new(""));
    let mut path = raw.trim().trim_end_matches(';').to_string();
    if let Some(rest) = path.strip_prefix("crate::") {
        path = rest.to_string();
        let crate_root = if importer.starts_with("src/") {
            PathBuf::from("src")
        } else {
            PathBuf::new()
        };
        return module_candidates(crate_root, &path);
    }
    if let Some(rest) = path.strip_prefix("self::") {
        path = rest.to_string();
        return module_candidates(current_dir.to_path_buf(), &path);
    }

    let mut base = current_dir.to_path_buf();
    let mut remaining = path.as_str();
    while let Some(rest) = remaining.strip_prefix("super::") {
        base = base.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
        remaining = rest;
    }
    if remaining != path {
        return module_candidates(base, remaining);
    }

    let mut candidates = module_candidates(current_dir.to_path_buf(), &path);
    candidates.extend(module_candidates(PathBuf::new(), &path));
    candidates
}

fn module_candidates(base: PathBuf, path: &str) -> Vec<PathBuf> {
    let segments: Vec<&str> = path
        .split("::")
        .filter(|segment| !segment.is_empty())
        .collect();
    let mut candidates = Vec::new();
    for length in (1..=segments.len()).rev() {
        let module = segments[..length].join("/");
        candidates.extend(path_candidates(&base.join(module), &["rs"]));
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_rust_module_imports_in_source_order() {
        let imports = imports("use crate::z::Thing;\nmod local;\nuse super::parent;\n");
        assert_eq!(
            imports.into_iter().map(|item| item.raw).collect::<Vec<_>>(),
            vec!["crate::z::Thing", "self::local", "super::parent"]
        );
    }

    #[test]
    fn resolves_crate_and_relative_modules() {
        let crate_candidates = candidates("src/main.rs", "crate::util");
        assert!(crate_candidates.contains(&PathBuf::from("src/util.rs")));
        let relative = candidates("src/bin/main.rs", "super::lib");
        assert!(relative.contains(&PathBuf::from("src/lib.rs")));
    }
}
