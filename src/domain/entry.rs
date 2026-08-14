//! Entry-point detection from conventions, manifests, and the import graph.

use std::collections::{HashMap, HashSet};

use crate::domain::content::ImportEdge;

/// A typeable file that can start a flow traversal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryCandidate {
    pub path: String,
    pub reason: &'static str,
}

/// Detect entry candidates in priority order. The first item is the default.
///
/// Paths not in `typeable` are dropped. Duplicates keep the earliest reason.
pub fn detect_entry_candidates(
    typeable: &[String],
    edges: &[ImportEdge],
    manifest_hints: &[EntryCandidate],
) -> Vec<EntryCandidate> {
    let typeable_set: HashSet<&str> = typeable.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    let mut push = |path: String, reason: &'static str| {
        if typeable_set.contains(path.as_str()) && seen.insert(path.clone()) {
            out.push(EntryCandidate { path, reason });
        }
    };

    for hint in manifest_hints {
        push(hint.path.clone(), hint.reason);
    }

    let mut exec: Vec<(String, &'static str)> = typeable
        .iter()
        .filter_map(|path| exec_reason(path).map(|reason| (path.clone(), reason)))
        .collect();
    exec.sort_by(|a, b| exec_rank(&a.0).cmp(&exec_rank(&b.0)).then(a.0.cmp(&b.0)));
    for (path, reason) in exec {
        push(path, reason);
    }

    let mut apps: Vec<(String, &'static str)> = typeable
        .iter()
        .filter_map(|path| app_reason(path).map(|reason| (path.clone(), reason)))
        .collect();
    apps.sort_by(|a, b| app_rank(&a.0).cmp(&app_rank(&b.0)).then(a.0.cmp(&b.0)));
    for (path, reason) in apps {
        push(path, reason);
    }

    for path in ["src/lib.rs", "lib.rs", "src/mod.rs"] {
        push(path.to_string(), "crate root");
    }

    if let Some(estimated) = graph_estimate(typeable, edges) {
        push(estimated, "graph");
    }

    out
}

fn exec_reason(path: &str) -> Option<&'static str> {
    if path == "src/main.rs"
        || is_src_bin_rs(path)
        || path == "main.go"
        || is_cmd_main_go(path)
        || path == "__main__.py"
        || path == "main.py"
        || path == "manage.py"
        || path == "app.py"
    {
        Some("bin")
    } else {
        None
    }
}

fn exec_rank(path: &str) -> u8 {
    match path {
        "src/main.rs" => 0,
        p if is_src_bin_rs(p) => 1,
        "main.go" => 2,
        p if is_cmd_main_go(p) => 3,
        "__main__.py" => 4,
        "main.py" => 5,
        "manage.py" => 6,
        "app.py" => 7,
        _ => 8,
    }
}

fn is_src_bin_rs(path: &str) -> bool {
    path.strip_prefix("src/bin/")
        .is_some_and(|rest| rest.ends_with(".rs") && !rest.contains('/'))
}

fn is_cmd_main_go(path: &str) -> bool {
    path.starts_with("cmd/") && path.ends_with("/main.go")
}

fn app_reason(path: &str) -> Option<&'static str> {
    if matches!(
        path,
        "src/index.ts"
            | "src/index.tsx"
            | "src/index.js"
            | "src/index.jsx"
            | "src/main.ts"
            | "src/main.tsx"
            | "src/main.js"
            | "src/main.jsx"
            | "index.ts"
            | "index.tsx"
            | "index.js"
            | "index.jsx"
            | "app/page.tsx"
            | "pages/index.ts"
            | "pages/index.tsx"
            | "pages/index.js"
            | "pages/index.jsx"
    ) {
        Some("app")
    } else {
        None
    }
}

fn app_rank(path: &str) -> u8 {
    match path {
        "src/index.ts" | "src/index.tsx" | "src/index.js" | "src/index.jsx" => 0,
        "src/main.ts" | "src/main.tsx" | "src/main.js" | "src/main.jsx" => 1,
        "index.ts" | "index.tsx" | "index.js" | "index.jsx" => 2,
        "app/page.tsx" => 3,
        p if p.starts_with("pages/index.") => 4,
        _ => 5,
    }
}

fn graph_estimate(typeable: &[String], edges: &[ImportEdge]) -> Option<String> {
    if typeable.is_empty() {
        return None;
    }
    let path_set: HashSet<&str> = typeable.iter().map(String::as_str).collect();
    let mut imported: HashSet<&str> = HashSet::new();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in edges {
        if path_set.contains(edge.importer.as_str()) && path_set.contains(edge.imported.as_str()) {
            imported.insert(edge.imported.as_str());
            let deps = adjacency.entry(edge.importer.as_str()).or_default();
            if !deps.contains(&edge.imported.as_str()) {
                deps.push(edge.imported.as_str());
            }
        }
    }

    let roots: Vec<&str> = typeable
        .iter()
        .map(String::as_str)
        .filter(|path| !imported.contains(path))
        .collect();
    let candidates = if roots.is_empty() {
        return None;
    } else {
        roots
    };

    candidates
        .into_iter()
        .max_by(|a, b| {
            reachable_count(a, &adjacency)
                .cmp(&reachable_count(b, &adjacency))
                .then_with(|| path_depth(b).cmp(&path_depth(a)))
                .then_with(|| (*b).cmp(*a))
        })
        .map(str::to_string)
}

fn path_depth(path: &str) -> usize {
    path.bytes().filter(|b| *b == b'/').count()
}

fn reachable_count(start: &str, adjacency: &HashMap<&str, Vec<&str>>) -> usize {
    let mut visited = HashSet::new();
    let mut stack = vec![start];
    while let Some(path) = stack.pop() {
        if !visited.insert(path) {
            continue;
        }
        if let Some(deps) = adjacency.get(path) {
            stack.extend(deps.iter().copied());
        }
    }
    visited.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn edge(importer: &str, imported: &str) -> ImportEdge {
        ImportEdge {
            importer: importer.into(),
            imported: imported.into(),
            decl_line: 1,
            first_use_line: None,
            raw: format!("import {imported}"),
        }
    }

    #[test]
    fn rust_prefers_manifest_then_main_then_lib() {
        let typeable = paths(&["src/lib.rs", "src/main.rs", "src/app/mod.rs"]);
        let found = detect_entry_candidates(
            &typeable,
            &[],
            &[EntryCandidate {
                path: "src/main.rs".into(),
                reason: "bin (Cargo.toml)",
            }],
        );
        assert_eq!(found[0].path, "src/main.rs");
        assert_eq!(found[0].reason, "bin (Cargo.toml)");
        assert!(found
            .iter()
            .any(|c| c.path == "src/lib.rs" && c.reason == "crate root"));
    }

    #[test]
    fn typescript_prefers_src_index() {
        let typeable = paths(&["src/util.ts", "src/index.ts", "src/lib.ts"]);
        let found = detect_entry_candidates(&typeable, &[], &[]);
        assert_eq!(found[0].path, "src/index.ts");
        assert_eq!(found[0].reason, "app");
    }

    #[test]
    fn python_prefers_main_py() {
        let typeable = paths(&["pkg/util.py", "main.py", "app.py"]);
        let found = detect_entry_candidates(&typeable, &[], &[]);
        assert_eq!(found[0].path, "main.py");
    }

    #[test]
    fn drops_paths_that_are_not_typeable() {
        let typeable = paths(&["src/lib.rs"]);
        let found = detect_entry_candidates(
            &typeable,
            &[],
            &[EntryCandidate {
                path: "src/main.rs".into(),
                reason: "bin (Cargo.toml)",
            }],
        );
        assert!(found.iter().all(|c| c.path != "src/main.rs"));
        assert_eq!(found[0].path, "src/lib.rs");
    }

    #[test]
    fn graph_estimate_picks_root_with_most_reach() {
        let typeable = paths(&["a.ts", "b.ts", "c.ts", "orphan.ts"]);
        let edges = vec![edge("a.ts", "b.ts"), edge("b.ts", "c.ts")];
        let found = detect_entry_candidates(&typeable, &edges, &[]);
        let graph = found.iter().find(|c| c.reason == "graph").unwrap();
        assert_eq!(graph.path, "a.ts");
    }

    #[test]
    fn graph_estimate_breaks_ties_by_shallow_then_lex() {
        let typeable = paths(&["z.ts", "src/a.ts"]);
        let found = detect_entry_candidates(&typeable, &[], &[]);
        let graph = found.iter().find(|c| c.reason == "graph").unwrap();
        assert_eq!(graph.path, "z.ts");
    }

    #[test]
    fn graph_estimate_skipped_when_every_file_is_imported() {
        let typeable = paths(&["a.ts", "b.ts"]);
        let edges = vec![edge("a.ts", "b.ts"), edge("b.ts", "a.ts")];
        let found = detect_entry_candidates(&typeable, &edges, &[]);
        assert!(found.iter().all(|c| c.reason != "graph"));
    }
}
