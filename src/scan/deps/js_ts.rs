//! Import extraction and relative-module resolution for JavaScript/TypeScript.

use std::path::{Path, PathBuf};

use super::path_candidates;
use super::ImportSpec;

pub(super) fn imports(source: &str) -> Vec<ImportSpec> {
    let mut result = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        let display = trimmed.to_string();
        let mut raws = Vec::new();
        if trimmed.starts_with("import ")
            || trimmed.starts_with("import\t")
            || trimmed.starts_with("export ")
        {
            if let Some(raw) = quoted_after_keyword(trimmed, "from") {
                raws.push(raw);
            } else if trimmed.starts_with("import") {
                if let Some(raw) = quoted_after_keyword(trimmed, "import") {
                    raws.push(raw);
                }
            }
        }
        if let Some(raw) = quoted_after_keyword(trimmed, "require(") {
            raws.push(raw);
        }
        if trimmed.starts_with("import(") {
            if let Some(raw) = quoted_after_keyword(trimmed, "import(") {
                raws.push(raw);
            }
        }
        let bindings = js_bindings(trimmed);
        for raw in raws {
            result.push(ImportSpec {
                raw,
                line: index + 1,
                bindings: bindings.clone(),
                display: display.clone(),
            });
        }
    }
    result
}

fn js_bindings(trimmed: &str) -> Vec<String> {
    if trimmed.starts_with("import(") {
        return Vec::new();
    }
    if let Some(idx) = trimmed.find("require(") {
        return assignment_name(&trimmed[..idx]);
    }
    let clause = if let Some(rest) = trimmed.strip_prefix("import ") {
        rest.split(" from ").next().unwrap_or("").trim()
    } else if trimmed.starts_with("export ") && trimmed.contains(" from ") {
        trimmed
            .strip_prefix("export ")
            .unwrap_or(trimmed)
            .split(" from ")
            .next()
            .unwrap_or("")
            .trim()
    } else {
        return Vec::new();
    };
    if clause.is_empty() || clause.starts_with(['\'', '"']) {
        return Vec::new();
    }
    parse_js_import_clause(clause)
}

fn assignment_name(prefix: &str) -> Vec<String> {
    let left = prefix.trim().trim_end_matches('=').trim();
    let name = left
        .rsplit([' ', '\t'])
        .next()
        .unwrap_or("")
        .trim_end_matches(';');
    if name.is_empty() || matches!(name, "const" | "let" | "var") {
        Vec::new()
    } else {
        vec![name.to_string()]
    }
}

fn parse_js_import_clause(clause: &str) -> Vec<String> {
    let mut bindings = Vec::new();
    if let Some(start) = clause.find('{') {
        if let Some(end) = clause[start..].find('}') {
            bindings.extend(parse_named(&clause[start + 1..start + end]));
        }
        let before = clause[..start].trim().trim_end_matches(',').trim();
        if !before.is_empty() && !before.starts_with('*') {
            bindings.insert(0, before.to_string());
        } else if let Some(ns) = namespace_alias(before) {
            bindings.insert(0, ns);
        }
    } else if let Some(ns) = namespace_alias(clause) {
        bindings.push(ns);
    } else {
        let ident = clause
            .split(|c: char| c == ',' || c.is_whitespace())
            .next()
            .unwrap_or("");
        if !ident.is_empty() && ident != "*" {
            bindings.push(ident.to_string());
        }
    }
    bindings
}

fn namespace_alias(clause: &str) -> Option<String> {
    let rest = clause.trim().strip_prefix('*')?.trim();
    let alias = rest.strip_prefix("as ")?.trim();
    let alias = alias
        .split(|c: char| c == ',' || c.is_whitespace())
        .next()
        .unwrap_or("")
        .trim();
    if alias.is_empty() {
        None
    } else {
        Some(alias.to_string())
    }
}

fn parse_named(inside: &str) -> Vec<String> {
    inside
        .split(',')
        .filter_map(|item| {
            let item = item.trim();
            if item.is_empty() {
                return None;
            }
            let item = item.strip_prefix("type ").unwrap_or(item).trim();
            let name = if let Some((_, alias)) = item.rsplit_once(" as ") {
                alias.trim()
            } else {
                item.trim()
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
    let raw = raw.split(['?', '#']).next().unwrap_or(raw).trim();
    if !raw.starts_with('.') {
        return Vec::new();
    }
    let importer_path = Path::new(importer);
    let current_dir = importer_path.parent().unwrap_or_else(|| Path::new(""));
    let path = current_dir.join(raw);
    path_candidates(&path, &["ts", "tsx", "js", "jsx"])
}

fn quoted_after_keyword(line: &str, keyword: &str) -> Option<String> {
    let start = line.find(keyword)? + keyword.len();
    let tail = &line[start..];
    let quote_index = tail.find(['\'', '"'])?;
    let quote = tail.as_bytes()[quote_index] as char;
    let value = &tail[quote_index + 1..];
    let end = value.find(quote)?;
    Some(value[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_relative_and_commonjs_imports() {
        let imports = imports(
            "import value from './value';\nexport * from \"./other\";\nconst x = require('./cjs');",
        );
        assert_eq!(
            imports
                .iter()
                .map(|item| item.raw.as_str())
                .collect::<Vec<_>>(),
            vec!["./value", "./other", "./cjs"]
        );
        assert_eq!(imports[0].bindings, vec!["value"]);
        assert!(imports[1].bindings.is_empty());
        assert_eq!(imports[2].bindings, vec!["x"]);
        assert_eq!(imports[0].line, 1);
    }

    #[test]
    fn extracts_named_namespace_and_side_effect_bindings() {
        let imports = imports(
            "import { a, b as c } from './x';\nimport * as ns from './y';\nimport './z';\n",
        );
        assert_eq!(imports[0].bindings, vec!["a", "c"]);
        assert_eq!(imports[1].bindings, vec!["ns"]);
        assert!(imports[2].bindings.is_empty());
    }

    #[test]
    fn ignores_external_modules() {
        assert!(candidates("src/main.ts", "react").is_empty());
        assert!(candidates("src/main.ts", "./util").contains(&PathBuf::from("src/util.ts")));
    }
}
