//! Pure dependency-ordering logic.

use std::collections::{HashMap, HashSet};

use crate::config::{DependencyDirection, ProgressMode};
use crate::domain::content::{FileStatus, ImportEdge, RepoProgress};

/// How a flow step was reached from its importer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowVia {
    pub importer: String,
    pub line: usize,
    pub raw: String,
}

/// One file in a computed flow order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowStep {
    pub path: String,
    pub via: Option<FlowVia>,
    /// `false` = not reachable from the entry (appended in path order).
    pub reachable: bool,
}

/// Cached traversal of typeable files from an entry point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowOrder {
    pub entry: String,
    pub steps: Vec<FlowStep>,
    index: HashMap<String, usize>,
}

impl FlowOrder {
    pub fn new(entry: String, steps: Vec<FlowStep>) -> Self {
        let index = steps
            .iter()
            .enumerate()
            .map(|(i, step)| (step.path.clone(), i))
            .collect();
        Self {
            entry,
            steps,
            index,
        }
    }

    /// 1-based index in the full order, only for reachable files.
    pub fn step_number(&self, path: &str) -> Option<usize> {
        let index = *self.index.get(path)?;
        self.steps.get(index).filter(|step| step.reachable)?;
        Some(index + 1)
    }

    pub fn via(&self, path: &str) -> Option<&FlowVia> {
        let index = *self.index.get(path)?;
        self.steps.get(index).and_then(|step| step.via.as_ref())
    }

    pub fn is_reachable(&self, path: &str) -> Option<bool> {
        self.index
            .get(path)
            .and_then(|index| self.steps.get(*index))
            .map(|step| step.reachable)
    }

    pub fn reachable_total(&self) -> usize {
        self.steps.iter().filter(|step| step.reachable).count()
    }

    pub fn reachable_done(&self, progress: &RepoProgress) -> usize {
        self.steps
            .iter()
            .filter(|step| {
                step.reachable
                    && progress.files.iter().any(|file| {
                        file.relative_path == step.path && file.derive_status() == FileStatus::Done
                    })
            })
            .count()
    }

    /// First incomplete file in flow order (including unreachable tail).
    pub fn next_step<'a>(&'a self, progress: &RepoProgress) -> Option<&'a FlowStep> {
        self.steps.iter().find(|step| {
            progress.files.iter().any(|file| {
                file.relative_path == step.path && file.derive_status() == FileStatus::Todo
            })
        })
    }
}

/// Return typeable files in dependency traversal order.
///
/// The first reachable path is traversed from `entry`; all other typeable
/// paths are appended in lexicographic order with `reachable = false`.
pub fn order_files(
    edges: &[ImportEdge],
    entry: Option<&str>,
    direction: DependencyDirection,
    typeable_paths: &[String],
) -> Vec<FlowStep> {
    let mut paths: Vec<String> = typeable_paths.to_vec();
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Vec::new();
    }

    let path_set: HashSet<&str> = paths.iter().map(String::as_str).collect();
    let mut adjacency: HashMap<&str, Vec<&ImportEdge>> = HashMap::new();
    for edge in edges {
        if path_set.contains(edge.importer.as_str()) && path_set.contains(edge.imported.as_str()) {
            adjacency
                .entry(edge.importer.as_str())
                .or_default()
                .push(edge);
        }
    }
    for deps in adjacency.values_mut() {
        deps.sort_by_key(|edge| {
            (
                edge.first_use_line.unwrap_or(usize::MAX),
                edge.decl_line,
                edge.imported.as_str(),
            )
        });
    }

    let start = entry
        .filter(|path| path_set.contains(path))
        .or_else(|| paths.first().map(String::as_str));
    let Some(start) = start else {
        return paths
            .into_iter()
            .map(|path| FlowStep {
                path,
                via: None,
                reachable: false,
            })
            .collect();
    };

    let mut ordered = Vec::with_capacity(paths.len());
    let mut visited = HashSet::new();
    visit(
        start,
        None,
        direction,
        &adjacency,
        &mut visited,
        &mut ordered,
    );

    for path in paths {
        if !visited.contains(&path) {
            visited.insert(path.clone());
            ordered.push(FlowStep {
                path,
                via: None,
                reachable: false,
            });
        }
    }
    ordered
}

/// Whether the configured progress mode follows import flow.
pub fn uses_flow_mode(mode: ProgressMode) -> bool {
    mode == ProgressMode::Flow
}

fn visit<'a>(
    path: &'a str,
    via: Option<FlowVia>,
    direction: DependencyDirection,
    adjacency: &HashMap<&'a str, Vec<&'a ImportEdge>>,
    visited: &mut HashSet<String>,
    ordered: &mut Vec<FlowStep>,
) {
    if !visited.insert(path.to_string()) {
        return;
    }

    if direction == DependencyDirection::TopDown {
        ordered.push(FlowStep {
            path: path.to_string(),
            via,
            reachable: true,
        });
        visit_children(path, direction, adjacency, visited, ordered);
        return;
    }

    visit_children(path, direction, adjacency, visited, ordered);
    ordered.push(FlowStep {
        path: path.to_string(),
        via,
        reachable: true,
    });
}

fn visit_children<'a>(
    path: &'a str,
    direction: DependencyDirection,
    adjacency: &HashMap<&'a str, Vec<&'a ImportEdge>>,
    visited: &mut HashSet<String>,
    ordered: &mut Vec<FlowStep>,
) {
    if let Some(dependencies) = adjacency.get(path) {
        for edge in dependencies {
            let child_via = FlowVia {
                importer: path.to_string(),
                line: edge.decl_line,
                raw: edge.raw.clone(),
            };
            visit(
                edge.imported.as_str(),
                Some(child_via),
                direction,
                adjacency,
                visited,
                ordered,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn edge(
        importer: &str,
        imported: &str,
        decl_line: usize,
        first_use_line: Option<usize>,
        raw: &str,
    ) -> ImportEdge {
        ImportEdge {
            importer: importer.into(),
            imported: imported.into(),
            decl_line,
            first_use_line,
            raw: raw.into(),
        }
    }

    fn step_paths(steps: &[FlowStep]) -> Vec<String> {
        steps.iter().map(|step| step.path.clone()).collect()
    }

    #[test]
    fn top_down_orders_siblings_by_first_use() {
        let edges = vec![
            edge("main.rs", "a.rs", 1, Some(20), "mod a;"),
            edge("main.rs", "z.rs", 2, Some(10), "mod z;"),
        ];
        let ordered = order_files(
            &edges,
            Some("main.rs"),
            DependencyDirection::TopDown,
            &paths(&["main.rs", "z.rs", "a.rs"]),
        );
        assert_eq!(step_paths(&ordered), paths(&["main.rs", "z.rs", "a.rs"]));
        assert_eq!(ordered[1].via.as_ref().map(|via| via.line), Some(2));
        assert_eq!(
            ordered[1].via.as_ref().map(|via| via.raw.as_str()),
            Some("mod z;")
        );
        assert!(ordered.iter().all(|step| step.reachable));
    }

    #[test]
    fn unused_imports_fall_back_to_declaration_order() {
        let edges = vec![
            edge("lib.rs", "z.rs", 3, None, "pub mod z;"),
            edge("lib.rs", "a.rs", 1, None, "pub mod a;"),
            edge("lib.rs", "m.rs", 2, None, "pub mod m;"),
        ];
        let ordered = order_files(
            &edges,
            Some("lib.rs"),
            DependencyDirection::TopDown,
            &paths(&["lib.rs", "a.rs", "m.rs", "z.rs"]),
        );
        assert_eq!(
            step_paths(&ordered),
            paths(&["lib.rs", "a.rs", "m.rs", "z.rs"])
        );
    }

    #[test]
    fn bottom_up_places_dependencies_first() {
        let edges = vec![
            edge("main.py", "pkg/a.py", 1, Some(3), "from pkg import a"),
            edge("pkg/a.py", "pkg/b.py", 1, Some(2), "from pkg import b"),
        ];
        let ordered = order_files(
            &edges,
            Some("main.py"),
            DependencyDirection::BottomUp,
            &paths(&["main.py", "pkg/a.py", "pkg/b.py"]),
        );
        assert_eq!(
            step_paths(&ordered),
            paths(&["pkg/b.py", "pkg/a.py", "main.py"])
        );
        assert_eq!(
            ordered[0].via.as_ref().map(|via| via.importer.as_str()),
            Some("pkg/a.py")
        );
        assert!(ordered[2].via.is_none());
    }

    #[test]
    fn cycles_are_visited_once() {
        let edges = vec![
            edge("a.ts", "b.ts", 1, Some(4), "import { b } from './b'"),
            edge("b.ts", "a.ts", 1, Some(4), "import { a } from './a'"),
        ];
        let ordered = order_files(
            &edges,
            Some("a.ts"),
            DependencyDirection::TopDown,
            &paths(&["a.ts", "b.ts"]),
        );
        assert_eq!(step_paths(&ordered), paths(&["a.ts", "b.ts"]));
        assert_eq!(ordered.len(), 2);
    }

    #[test]
    fn unreachable_paths_are_appended_sorted() {
        let edges = vec![edge("main.rs", "dep.rs", 1, Some(2), "mod dep;")];
        let ordered = order_files(
            &edges,
            Some("main.rs"),
            DependencyDirection::TopDown,
            &paths(&["z.rs", "main.rs", "orphan.rs", "dep.rs"]),
        );
        assert_eq!(
            step_paths(&ordered),
            paths(&["main.rs", "dep.rs", "orphan.rs", "z.rs"])
        );
        assert!(ordered[0].reachable);
        assert!(ordered[1].reachable);
        assert!(!ordered[2].reachable);
        assert!(!ordered[3].reachable);
        assert!(ordered[2].via.is_none());
    }

    #[test]
    fn invalid_entry_falls_back_to_path_order() {
        let ordered = order_files(
            &[],
            Some("missing.rs"),
            DependencyDirection::TopDown,
            &paths(&["b.rs", "a.rs"]),
        );
        assert_eq!(step_paths(&ordered), paths(&["a.rs", "b.rs"]));
        assert!(ordered[0].reachable);
        assert!(!ordered[1].reachable);
    }

    #[test]
    fn flow_order_numbers_skip_unreachable() {
        let steps = vec![
            FlowStep {
                path: "main.rs".into(),
                via: None,
                reachable: true,
            },
            FlowStep {
                path: "dep.rs".into(),
                via: Some(FlowVia {
                    importer: "main.rs".into(),
                    line: 1,
                    raw: "mod dep;".into(),
                }),
                reachable: true,
            },
            FlowStep {
                path: "orphan.rs".into(),
                via: None,
                reachable: false,
            },
        ];
        let order = FlowOrder::new("main.rs".into(), steps);
        assert_eq!(order.step_number("main.rs"), Some(1));
        assert_eq!(order.step_number("dep.rs"), Some(2));
        assert_eq!(order.step_number("orphan.rs"), None);
        assert_eq!(order.via("dep.rs").map(|via| via.line), Some(1));
        assert_eq!(order.reachable_total(), 2);
    }
}
