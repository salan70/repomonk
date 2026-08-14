//! Manifest-derived entry hints (`Cargo.toml`, `package.json`).

use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::domain::entry::EntryCandidate;

/// Read entry-point hints from well-known manifests. Failures yield an empty list.
pub fn read_entry_hints(root: &Path) -> Vec<EntryCandidate> {
    let mut hints = Vec::new();
    hints.extend(cargo_hints(root));
    hints.extend(package_json_hints(root));
    hints
}

fn cargo_hints(root: &Path) -> Vec<EntryCandidate> {
    let path = root.join("Cargo.toml");
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(parsed) = toml::from_str::<CargoToml>(&text) else {
        return Vec::new();
    };
    let mut hints = vec![EntryCandidate {
        path: "src/main.rs".into(),
        reason: "bin (Cargo.toml)",
    }];
    for bin in parsed.bin {
        if let Some(bin_path) = bin.path {
            let normalized = bin_path.replace('\\', "/");
            if normalized != "src/main.rs" {
                hints.push(EntryCandidate {
                    path: normalized,
                    reason: "bin (Cargo.toml)",
                });
            }
        }
    }
    hints
}

#[derive(Debug, Default, Deserialize)]
struct CargoToml {
    #[serde(default)]
    bin: Vec<CargoBin>,
}

#[derive(Debug, Default, Deserialize)]
struct CargoBin {
    path: Option<String>,
}

fn package_json_hints(root: &Path) -> Vec<EntryCandidate> {
    let path = root.join("package.json");
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut hints = Vec::new();
    if let Some(main) = json_string_value(&text, "main") {
        hints.push(EntryCandidate {
            path: strip_dot_slash(&main),
            reason: "main (package.json)",
        });
    }
    for bin in json_bin_values(&text) {
        hints.push(EntryCandidate {
            path: strip_dot_slash(&bin),
            reason: "bin (package.json)",
        });
    }
    hints
}

fn strip_dot_slash(path: &str) -> String {
    path.trim().trim_start_matches("./").replace('\\', "/")
}

fn json_string_value(text: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\"");
    let start = text.find(&pattern)?;
    let rest = text.get(start + pattern.len()..)?.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    parse_json_string(rest)
}

fn json_bin_values(text: &str) -> Vec<String> {
    let pattern = "\"bin\"";
    let Some(start) = text.find(pattern) else {
        return Vec::new();
    };
    let Some(rest) = text.get(start + pattern.len()..) else {
        return Vec::new();
    };
    let rest = rest.trim_start();
    let Some(rest) = rest.strip_prefix(':') else {
        return Vec::new();
    };
    let rest = rest.trim_start();
    if rest.starts_with('"') {
        return parse_json_string(rest).into_iter().collect();
    }
    if !rest.starts_with('{') {
        return Vec::new();
    }
    let Some(end) = rest.find('}') else {
        return Vec::new();
    };
    let object = &rest[1..end];
    let mut values = Vec::new();
    let mut remaining = object;
    while let Some(colon) = remaining.find(':') {
        remaining = remaining[colon + 1..].trim_start();
        if let Some(value) = parse_json_string(remaining) {
            let consumed = value.len() + 2;
            values.push(value);
            remaining = remaining.get(consumed..).unwrap_or("");
        } else if remaining.is_empty() {
            break;
        } else {
            remaining = remaining.get(1..).unwrap_or("");
        }
    }
    values
}

fn parse_json_string(s: &str) -> Option<String> {
    let s = s.trim_start();
    let mut chars = s.strip_prefix('"')?.chars();
    let mut out = String::new();
    loop {
        match chars.next()? {
            '"' => return Some(out),
            '\\' => {
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            }
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn cargo_toml_adds_default_main_and_bin_paths() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            r#"
[package]
name = "demo"
version = "0.1.0"

[[bin]]
name = "tool"
path = "src/bin/tool.rs"
"#,
        )
        .unwrap();
        let hints = read_entry_hints(dir.path());
        let paths: Vec<_> = hints.iter().map(|h| h.path.as_str()).collect();
        assert!(paths.contains(&"src/main.rs"));
        assert!(paths.contains(&"src/bin/tool.rs"));
        assert!(hints.iter().all(|h| h.reason == "bin (Cargo.toml)"));
    }

    #[test]
    fn missing_manifest_yields_empty() {
        let dir = tempdir().unwrap();
        assert!(read_entry_hints(dir.path()).is_empty());
    }

    #[test]
    fn package_json_reads_main_and_bin_object() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"
{
  "name": "demo",
  "main": "src/index.ts",
  "bin": { "cli": "bin/cli.js", "other": "./scripts/other.js" }
}
"#,
        )
        .unwrap();
        let hints = read_entry_hints(dir.path());
        assert!(hints
            .iter()
            .any(|h| h.path == "src/index.ts" && h.reason == "main (package.json)"));
        assert!(hints
            .iter()
            .any(|h| h.path == "bin/cli.js" && h.reason == "bin (package.json)"));
        assert!(hints.iter().any(|h| h.path == "scripts/other.js"));
    }

    #[test]
    fn invalid_manifest_is_ignored() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "not = [ valid").unwrap();
        assert!(read_entry_hints(dir.path()).is_empty());
    }
}
