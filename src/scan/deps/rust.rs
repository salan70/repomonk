//! Lightweight Rust module/import resolver.

use std::path::{Path, PathBuf};

use super::path_candidates;
use super::ImportSpec;

pub(super) fn imports(source: &str) -> Vec<ImportSpec> {
    let mut result = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let display = line.trim().to_string();
        let body = display.strip_prefix("pub ").unwrap_or(display.as_str());
        if let Some(rest) = body.strip_prefix("use ") {
            let stmt = rest.split(';').next().unwrap_or(rest).trim();
            let (path_part, brace_part) = split_use_path(stmt);
            let (path, alias) = split_as(&path_part);
            let bindings = if let Some(inside) = brace_part {
                parse_use_bindings(&inside)
            } else if path.ends_with('*') {
                Vec::new()
            } else if let Some(alias) = alias {
                vec![alias]
            } else {
                path.rsplit("::")
                    .next()
                    .filter(|name| !name.is_empty() && *name != "*")
                    .map(str::to_string)
                    .into_iter()
                    .collect()
            };
            let raw = path
                .trim_end_matches("::*")
                .trim_end_matches("::")
                .trim()
                .to_string();
            if !raw.is_empty() {
                result.push(ImportSpec {
                    raw,
                    line: index + 1,
                    bindings,
                    display: display.clone(),
                });
            }
        } else if let Some(rest) = body.strip_prefix("mod ") {
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
                    line: index + 1,
                    bindings: vec![name.to_string()],
                    display: display.clone(),
                });
            }
        }
    }
    result
}

fn split_use_path(stmt: &str) -> (String, Option<String>) {
    if let Some(idx) = stmt.find('{') {
        let path = stmt[..idx].trim().trim_end_matches("::").to_string();
        let inside = stmt[idx + 1..].split('}').next().unwrap_or("").to_string();
        (path, Some(inside))
    } else {
        (stmt.to_string(), None)
    }
}

fn split_as(s: &str) -> (String, Option<String>) {
    if let Some((left, right)) = s.rsplit_once(" as ") {
        (left.trim().to_string(), Some(right.trim().to_string()))
    } else {
        (s.trim().to_string(), None)
    }
}

fn parse_use_bindings(inside: &str) -> Vec<String> {
    inside
        .split(',')
        .filter_map(|item| {
            let item = item.trim();
            if item.is_empty() || item == "*" {
                return None;
            }
            let name = if let Some((_, alias)) = item.rsplit_once(" as ") {
                alias.trim()
            } else {
                item.rsplit("::").next().unwrap_or(item).trim()
            };
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect()
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
            imports
                .iter()
                .map(|item| item.raw.as_str())
                .collect::<Vec<_>>(),
            vec!["crate::z::Thing", "self::local", "super::parent"]
        );
        assert_eq!(imports[0].bindings, vec!["Thing"]);
        assert_eq!(imports[1].bindings, vec!["local"]);
        assert_eq!(imports[0].line, 1);
        assert_eq!(imports[1].line, 2);
    }

    #[test]
    fn extracts_grouped_and_aliased_bindings() {
        let imports = imports("use a::b::{C, D as E};\nuse a::b as alias;\n");
        assert_eq!(imports[0].raw, "a::b");
        assert_eq!(imports[0].bindings, vec!["C", "E"]);
        assert_eq!(imports[1].raw, "a::b");
        assert_eq!(imports[1].bindings, vec!["alias"]);
    }

    #[test]
    fn resolves_crate_and_relative_modules() {
        let crate_candidates = candidates("src/main.rs", "crate::util");
        assert!(crate_candidates.contains(&PathBuf::from("src/util.rs")));
        let relative = candidates("src/bin/main.rs", "super::lib");
        assert!(relative.contains(&PathBuf::from("src/lib.rs")));
    }
}
