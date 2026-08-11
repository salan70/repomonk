//! Pure dependency-ordering logic.

use std::collections::{HashMap, HashSet};

use crate::config::{DependencyDirection, ProgressMode};

/// Return typeable files in dependency traversal order.
///
/// Edges are `(importer, imported)` pairs. The first reachable path is
/// traversed from `entry`; all other typeable paths are appended in
/// lexicographic order. The returned order is always duplicate-free.
pub fn order_files(
    edges: &[(String, String)],
    entry: Option<&str>,
    direction: DependencyDirection,
    typeable_paths: &[String],
) -> Vec<String> {
    let mut paths: Vec<String> = typeable_paths.to_vec();
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return paths;
    }

    let path_set: HashSet<&str> = paths.iter().map(String::as_str).collect();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for (from, to) in edges {
        if path_set.contains(from.as_str()) && path_set.contains(to.as_str()) {
            let dependencies = adjacency.entry(from.as_str()).or_default();
            if !dependencies.contains(&to.as_str()) {
                dependencies.push(to.as_str());
            }
        }
    }

    let start = entry
        .filter(|path| path_set.contains(path))
        .or_else(|| paths.first().map(String::as_str));
    let Some(start) = start else {
        return paths;
    };

    let mut ordered = Vec::with_capacity(paths.len());
    let mut visited = HashSet::new();
    visit(start, direction, &adjacency, &mut visited, &mut ordered);

    for path in paths {
        if !visited.contains(&path) {
            visited.insert(path.clone());
            ordered.push(path);
        }
    }
    ordered
}

/// Whether the configured progress mode needs dependency ordering.
pub fn uses_dependency_mode(mode: ProgressMode) -> bool {
    mode == ProgressMode::Dependency
}

fn visit<'a>(
    path: &'a str,
    direction: DependencyDirection,
    adjacency: &HashMap<&'a str, Vec<&'a str>>,
    visited: &mut HashSet<String>,
    ordered: &mut Vec<String>,
) {
    if !visited.insert(path.to_string()) {
        return;
    }

    if direction == DependencyDirection::TopDown {
        ordered.push(path.to_string());
    }

    if let Some(dependencies) = adjacency.get(path) {
        for dependency in dependencies {
            visit(dependency, direction, adjacency, visited, ordered);
        }
    }

    if direction == DependencyDirection::BottomUp {
        ordered.push(path.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn top_down_preserves_import_order() {
        let edges = vec![
            ("main.rs".into(), "z.rs".into()),
            ("main.rs".into(), "a.rs".into()),
        ];
        assert_eq!(
            order_files(
                &edges,
                Some("main.rs"),
                DependencyDirection::TopDown,
                &paths(&["main.rs", "z.rs", "a.rs"]),
            ),
            paths(&["main.rs", "z.rs", "a.rs"])
        );
    }

    #[test]
    fn bottom_up_places_dependencies_first() {
        let edges = vec![
            ("main.py".into(), "pkg/a.py".into()),
            ("pkg/a.py".into(), "pkg/b.py".into()),
        ];
        assert_eq!(
            order_files(
                &edges,
                Some("main.py"),
                DependencyDirection::BottomUp,
                &paths(&["main.py", "pkg/a.py", "pkg/b.py"]),
            ),
            paths(&["pkg/b.py", "pkg/a.py", "main.py"])
        );
    }

    #[test]
    fn cycles_are_visited_once() {
        let edges = vec![
            ("a.ts".into(), "b.ts".into()),
            ("b.ts".into(), "a.ts".into()),
        ];
        let ordered = order_files(
            &edges,
            Some("a.ts"),
            DependencyDirection::TopDown,
            &paths(&["a.ts", "b.ts"]),
        );
        assert_eq!(ordered, paths(&["a.ts", "b.ts"]));
    }

    #[test]
    fn unreachable_paths_are_appended_sorted() {
        let edges = vec![("main.rs".into(), "dep.rs".into())];
        assert_eq!(
            order_files(
                &edges,
                Some("main.rs"),
                DependencyDirection::TopDown,
                &paths(&["z.rs", "main.rs", "orphan.rs", "dep.rs"]),
            ),
            paths(&["main.rs", "dep.rs", "orphan.rs", "z.rs"])
        );
    }

    #[test]
    fn invalid_entry_falls_back_to_path_order() {
        assert_eq!(
            order_files(
                &[],
                Some("missing.rs"),
                DependencyDirection::TopDown,
                &paths(&["b.rs", "a.rs"]),
            ),
            paths(&["a.rs", "b.rs"])
        );
    }
}
