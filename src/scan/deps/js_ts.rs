//! Import extraction and relative-module resolution for JavaScript/TypeScript.

use std::path::{Path, PathBuf};

use super::path_candidates;
use super::ImportSpec;

pub(super) fn imports(source: &str) -> Vec<ImportSpec> {
    let mut result = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("import ")
            || trimmed.starts_with("import\t")
            || trimmed.starts_with("export ")
        {
            if let Some(raw) = quoted_after_keyword(trimmed, "from") {
                result.push(ImportSpec { raw });
            } else if trimmed.starts_with("import") {
                if let Some(raw) = quoted_after_keyword(trimmed, "import") {
                    result.push(ImportSpec { raw });
                }
            }
        }
        if let Some(raw) = quoted_after_keyword(trimmed, "require(") {
            result.push(ImportSpec { raw });
        }
        if trimmed.starts_with("import(") {
            if let Some(raw) = quoted_after_keyword(trimmed, "import(") {
                result.push(ImportSpec { raw });
            }
        }
    }
    result
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
            imports.into_iter().map(|item| item.raw).collect::<Vec<_>>(),
            vec!["./value", "./other", "./cjs"]
        );
    }

    #[test]
    fn ignores_external_modules() {
        assert!(candidates("src/main.ts", "react").is_empty());
        assert!(candidates("src/main.ts", "./util").contains(&PathBuf::from("src/util.ts")));
    }
}
