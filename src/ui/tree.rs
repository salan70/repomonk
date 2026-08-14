//! File tree view model and rendering.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::domain::content::{FileStatus, RepoProgress};
use crate::domain::progress::directory_progress;
use crate::ui::theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRowKind {
    Dir { path: String },
    File { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub kind: TreeRowKind,
    pub depth: usize,
    /// Display name (file name or directory name without trailing slash).
    pub name: String,
    /// `(completed, total)` normalized line counts; `None` for skipped files.
    pub progress: Option<(usize, usize)>,
    /// File status; `None` for directories.
    pub status: Option<FileStatus>,
    /// Human-readable skip reason for skipped files.
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TreeView {
    pub rows: Vec<TreeRow>,
    pub selected: usize,
    pub recommend: Option<String>,
    pub title: String,
    pub collapsed: std::collections::HashSet<String>,
    /// Dependency traversal order, when dependency mode is active.
    pub dependency_order: Option<Vec<String>>,
    /// Repository-wide `(completed, total)` normalized line counts.
    pub overall: (usize, usize),
    pub hide_skipped: bool,
    /// Incremental file filter (`/` on Tree).
    pub filter: String,
    pub filter_editing: bool,
    /// Last drawn list height, used for half-page jumps.
    pub visible_rows: usize,
    /// Shown after Result Enter when the repository is fully done.
    pub repo_complete: bool,
    /// One-line status/error banner (e.g. from the File types overlay), cleared on next input.
    pub message: Option<String>,
}

impl TreeView {
    pub fn from_progress(repo_name: &str, progress: &RepoProgress, hide_skipped: bool) -> Self {
        Self::from_progress_with_order(repo_name, progress, hide_skipped, None)
    }

    pub fn from_progress_with_order(
        repo_name: &str,
        progress: &RepoProgress,
        hide_skipped: bool,
        dependency_order: Option<Vec<String>>,
    ) -> Self {
        let recommend = recommended_path(progress, dependency_order.as_deref());
        let rows = flatten(
            progress,
            &std::collections::HashSet::new(),
            hide_skipped,
            "",
        );
        Self {
            rows,
            selected: 0,
            recommend,
            title: repo_name.to_string(),
            collapsed: std::collections::HashSet::new(),
            dependency_order,
            overall: overall_progress(progress),
            hide_skipped,
            filter: String::new(),
            filter_editing: false,
            visible_rows: 1,
            repo_complete: false,
            message: None,
        }
    }

    pub fn refresh_rows(&mut self, progress: &RepoProgress) {
        self.recommend = recommended_path(progress, self.dependency_order.as_deref());
        self.overall = overall_progress(progress);
        let prev_path = self.selected_path();
        self.rows = flatten(progress, &self.collapsed, self.hide_skipped, &self.filter);
        if let Some(path) = prev_path {
            if let Some(idx) = self.rows.iter().position(|r| row_path(r) == path) {
                self.selected = idx;
            } else {
                self.selected = self.selected.min(self.rows.len().saturating_sub(1));
            }
        } else {
            self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        }
    }

    pub fn set_dependency_order(
        &mut self,
        progress: &RepoProgress,
        dependency_order: Option<Vec<String>>,
    ) {
        self.dependency_order = dependency_order;
        self.refresh_rows(progress);
    }

    pub fn dependency_order_number(&self, path: &str) -> Option<usize> {
        self.dependency_order
            .as_ref()
            .and_then(|order| order.iter().position(|item| item == path))
            .map(|index| index + 1)
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() || delta == 0 {
            return;
        }
        let len = self.rows.len() as isize;
        let direction = delta.signum();
        let mut next = (self.selected as isize + delta).rem_euclid(len);

        for _ in 0..self.rows.len() {
            if self.rows[next as usize]
                .status
                .is_some_and(|status| status != FileStatus::Skipped)
            {
                self.selected = next as usize;
                return;
            }
            next = (next + direction).rem_euclid(len);
        }
    }

    pub fn selected_file_path(&self) -> Option<String> {
        self.rows.get(self.selected).and_then(|r| match &r.kind {
            TreeRowKind::File { path } => Some(path.clone()),
            TreeRowKind::Dir { .. } => None,
        })
    }

    pub fn selected_dir_path(&self) -> Option<String> {
        self.rows.get(self.selected).and_then(|r| match &r.kind {
            TreeRowKind::Dir { path } => Some(path.clone()),
            TreeRowKind::File { .. } => None,
        })
    }

    pub fn selected_path(&self) -> Option<String> {
        self.rows.get(self.selected).map(row_path)
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.rows.len().saturating_sub(1);
    }

    pub fn page_by(&mut self, direction: isize) {
        let step = (self.visible_rows / 2).max(1) as isize * direction;
        self.move_by(step);
    }

    pub fn jump_to_path(&mut self, path: &str) -> bool {
        if let Some(idx) = self
            .rows
            .iter()
            .position(|r| matches!(&r.kind, TreeRowKind::File { path: p } if p == path))
        {
            self.selected = idx;
            true
        } else {
            false
        }
    }

    pub fn jump_recommend(&mut self) -> bool {
        self.recommend
            .clone()
            .is_some_and(|path| self.jump_to_path(&path))
    }

    pub fn next_match(&mut self, backward: bool) {
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len();
        for offset in 1..=len {
            let idx = if backward {
                (self.selected + len - offset) % len
            } else {
                (self.selected + offset) % len
            };
            if matches!(self.rows[idx].kind, TreeRowKind::File { .. }) {
                self.selected = idx;
                return;
            }
        }
    }

    pub fn begin_filter(&mut self) {
        self.filter_editing = true;
    }

    pub fn push_filter(&mut self, ch: char, progress: &RepoProgress) {
        self.filter.push(ch);
        self.refresh_rows(progress);
    }

    pub fn pop_filter(&mut self, progress: &RepoProgress) {
        self.filter.pop();
        self.refresh_rows(progress);
    }

    pub fn clear_filter(&mut self, progress: &RepoProgress) {
        self.filter.clear();
        self.filter_editing = false;
        self.refresh_rows(progress);
    }

    pub fn expand_dir(&mut self, progress: &RepoProgress) {
        if let Some(path) = self.selected_dir_path() {
            if self.collapsed.remove(&path) {
                self.refresh_rows(progress);
            }
        }
    }

    pub fn collapse_or_parent(&mut self, progress: &RepoProgress) {
        match self.rows.get(self.selected).map(|r| r.kind.clone()) {
            Some(TreeRowKind::Dir { path }) => {
                if self.collapsed.insert(path.clone()) {
                    self.refresh_rows(progress);
                } else if let Some(parent) = parent_path(&path) {
                    self.jump_dir(&parent);
                }
            }
            Some(TreeRowKind::File { path }) => {
                if let Some(parent) = parent_path(&path) {
                    self.jump_dir(&parent);
                }
            }
            None => {}
        }
    }

    fn jump_dir(&mut self, path: &str) {
        if let Some(idx) = self
            .rows
            .iter()
            .position(|r| matches!(&r.kind, TreeRowKind::Dir { path: p } if p == path))
        {
            self.selected = idx;
        }
    }

    pub fn toggle_collapse(&mut self, progress: &RepoProgress) {
        if let Some(TreeRow {
            kind: TreeRowKind::Dir { path },
            ..
        }) = self.rows.get(self.selected)
        {
            let path = path.clone();
            if !self.collapsed.remove(&path) {
                self.collapsed.insert(path);
            }
            self.refresh_rows(progress);
        }
    }
}

fn overall_progress(progress: &RepoProgress) -> (usize, usize) {
    let root = directory_progress(progress, "");
    (root.completed_lines, root.total_lines)
}

fn recommended_path(
    progress: &RepoProgress,
    dependency_order: Option<&[String]>,
) -> Option<String> {
    if let Some(order) = dependency_order {
        return order.iter().find_map(|path| {
            progress
                .files
                .iter()
                .find(|file| file.relative_path == *path)
                .filter(|file| file.derive_status() == FileStatus::Todo)
                .map(|_| path.clone())
        });
    }
    progress.recommend_path().map(str::to_string)
}

fn row_path(row: &TreeRow) -> String {
    match &row.kind {
        TreeRowKind::Dir { path } | TreeRowKind::File { path } => path.clone(),
    }
}

fn parent_path(path: &str) -> Option<String> {
    path.rsplit_once('/').map(|(parent, _)| parent.to_string())
}

fn flatten(
    progress: &RepoProgress,
    collapsed: &std::collections::HashSet<String>,
    hide_skipped: bool,
    filter: &str,
) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    // Build a simple path-sorted expansion.
    let mut dirs: Vec<String> = Vec::new();
    for f in &progress.files {
        if hide_skipped && f.derive_status() == FileStatus::Skipped {
            continue;
        }
        let parts: Vec<&str> = f.relative_path.split('/').collect();
        let mut acc = String::new();
        for (i, part) in parts.iter().enumerate() {
            if i + 1 == parts.len() {
                break;
            }
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(part);
            if !dirs.iter().any(|d| d == &acc) {
                dirs.push(acc.clone());
            }
        }
    }
    dirs.sort();

    // Emit via DFS on sorted unique paths.
    let mut emitted_dirs = std::collections::HashSet::new();
    for f in &progress.files {
        if hide_skipped && f.derive_status() == FileStatus::Skipped {
            continue;
        }
        let parts: Vec<&str> = f.relative_path.split('/').collect();
        let mut acc = String::new();
        let mut hidden = false;
        for (i, part) in parts.iter().enumerate() {
            let is_file = i + 1 == parts.len();
            if !is_file {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(part);
                if hidden {
                    continue;
                }
                if !emitted_dirs.contains(&acc) {
                    let dprog = directory_progress(progress, &acc);
                    rows.push(TreeRow {
                        kind: TreeRowKind::Dir { path: acc.clone() },
                        depth: i,
                        name: (*part).to_string(),
                        progress: Some((dprog.completed_lines, dprog.total_lines)),
                        status: None,
                        skip_reason: None,
                    });
                    emitted_dirs.insert(acc.clone());
                }
                if collapsed.contains(&acc) {
                    hidden = true;
                }
            } else if !hidden {
                let status = f.derive_status();
                let (progress_counts, skip_reason) = if status == FileStatus::Skipped {
                    (None, f.display_skip_reason())
                } else {
                    (Some((f.completed_lines(), f.total_lines())), None)
                };
                rows.push(TreeRow {
                    kind: TreeRowKind::File {
                        path: f.relative_path.clone(),
                    },
                    depth: i,
                    name: (*part).to_string(),
                    progress: progress_counts,
                    status: Some(status),
                    skip_reason,
                });
            }
        }
    }
    filter_rows(rows, filter)
}

fn file_matches_filter(path: &str, name: &str, q: &str) -> bool {
    let path = path.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    if path == q || path.ends_with(&format!("/{q}")) {
        return true;
    }
    if let Some(idx) = name.find(q) {
        return idx == 0 || matches!(name.as_bytes()[idx - 1], b'.' | b'/' | b'_' | b'-');
    }
    false
}

fn filter_rows(rows: Vec<TreeRow>, filter: &str) -> Vec<TreeRow> {
    let q = filter.trim().to_ascii_lowercase();
    if q.is_empty() {
        return rows;
    }
    let matching: Vec<String> = rows
        .iter()
        .filter_map(|row| match &row.kind {
            TreeRowKind::File { path } if file_matches_filter(path, &row.name, &q) => {
                Some(path.clone())
            }
            _ => None,
        })
        .collect();
    rows.into_iter()
        .filter(|row| match &row.kind {
            TreeRowKind::File { path } => matching.iter().any(|p| p == path),
            TreeRowKind::Dir { path } => matching
                .iter()
                .any(|file| file == path || file.starts_with(&format!("{path}/"))),
        })
        .collect()
}

pub fn draw_tree(frame: &mut Frame, area: Rect, view: &TreeView) {
    theme::fill_background(frame, area);
    let block = theme::bordered_block(theme::title_line(&view.title));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let filter_line = view.filter_editing || !view.filter.is_empty();
    let second_line = filter_line || view.message.is_some();
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if second_line {
            vec![
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ]
        } else {
            vec![
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ]
        })
        .split(inner);
    let list_pane = if second_line { panes[2] } else { panes[1] };
    let footer_pane = if second_line { panes[3] } else { panes[2] };

    // Header: repository-wide progress gauge.
    let (completed, total) = view.overall;
    let ratio = if total == 0 {
        0.0
    } else {
        completed as f64 / total as f64
    };
    let gauge = Gauge::default()
        .ratio(ratio)
        .use_unicode(true)
        .gauge_style(Style::default().fg(theme::BLUE).bg(theme::CURRENT_LINE_BG))
        .label(Span::styled(
            if view.repo_complete {
                format!("{completed}/{total} lines · 100% · repo complete")
            } else {
                format!("{completed}/{total} lines · {:.0}%", ratio * 100.0)
            },
            Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
        ));
    let gauge_area = Rect {
        x: panes[0].x + 1,
        y: panes[0].y,
        width: panes[0].width.saturating_sub(2),
        height: panes[0].height,
    };
    frame.render_widget(gauge, gauge_area);

    if filter_line {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" /", Style::default().fg(theme::CYAN)),
                Span::styled(
                    view.filter.clone(),
                    Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if view.filter_editing { "▌" } else { "" },
                    Style::default().fg(theme::CYAN),
                ),
            ])),
            panes[1],
        );
    } else if let Some(message) = &view.message {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {message}"),
                Style::default().fg(theme::MUTED),
            ))),
            panes[1],
        );
    }

    // Body: tree rows.
    let items: Vec<ListItem> = view
        .rows
        .iter()
        .map(|row| ListItem::new(row_line(row, view)))
        .collect();

    let mut state = ListState::default();
    if !view.rows.is_empty() {
        state.select(Some(view.selected));
    }
    let list = List::new(items)
        .highlight_style(Style::default().bg(theme::SELECTION_BG))
        .style(theme::base_style());
    frame.render_stateful_widget(list, list_pane, &mut state);

    let hints = if view.filter_editing {
        &[("n/N", "next"), ("Esc", "clear"), ("?", "help")][..]
    } else {
        &[
            ("Enter", "open"),
            ("j/k", "move"),
            ("Tab", "recommend"),
            ("t", "file types"),
            ("Esc", "back"),
            ("?", "help"),
        ][..]
    };
    frame.render_widget(Paragraph::new(theme::key_hints(hints)), footer_pane);
}

fn row_line(row: &TreeRow, view: &TreeView) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::raw(" ".repeat(1 + row.depth * 2)));

    match &row.kind {
        TreeRowKind::Dir { path } => {
            let arrow = if view.collapsed.contains(path) {
                "▸ "
            } else {
                "▾ "
            };
            spans.push(Span::styled(
                arrow.to_string(),
                Style::default().fg(theme::MUTED),
            ));
            spans.push(Span::styled(
                format!("{}/", row.name),
                Style::default()
                    .fg(theme::BLUE)
                    .add_modifier(Modifier::BOLD),
            ));
            if let Some((done, total)) = row.progress {
                spans.push(Span::styled(
                    format!("  [{done}/{total} lines]"),
                    Style::default().fg(theme::MUTED),
                ));
            }
        }
        TreeRowKind::File { path } => {
            if let Some(number) = view.dependency_order_number(path) {
                spans.push(Span::styled(
                    format!("{number:>3} "),
                    Style::default().fg(theme::CYAN),
                ));
            }
            let status = row.status.unwrap_or(FileStatus::Todo);
            let (mark, mark_color, name_color) = match status {
                FileStatus::Done => ("✓", theme::GREEN, theme::MUTED),
                FileStatus::Todo => ("○", theme::FG, theme::FG),
                FileStatus::Skipped => ("·", theme::MUTED, theme::MUTED),
            };
            spans.push(Span::styled(
                format!("{mark} "),
                Style::default().fg(mark_color),
            ));
            spans.push(Span::styled(
                row.name.clone(),
                Style::default().fg(name_color),
            ));
            if let Some(reason) = &row.skip_reason {
                spans.push(Span::styled(
                    format!("  ({reason})"),
                    Style::default()
                        .fg(theme::MUTED)
                        .add_modifier(Modifier::ITALIC),
                ));
            } else if let Some((done, total)) = row.progress {
                let color = if status == FileStatus::Done {
                    theme::GREEN
                } else {
                    theme::MUTED
                };
                spans.push(Span::styled(
                    format!("  [{done}/{total}]"),
                    Style::default().fg(color),
                ));
            }
            if view.recommend.as_deref() == Some(path.as_str()) {
                spans.push(Span::styled(
                    "  ▸ recommend",
                    Style::default()
                        .fg(theme::CYAN)
                        .add_modifier(Modifier::BOLD),
                ));
            }
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::content::{
        Chunk, ChunkCompletion, ChunkProgress, FileProgress, FileStatus, RepoProgress,
    };

    fn progress() -> RepoProgress {
        let file = |path: &str, body: &str| FileProgress {
            relative_path: path.into(),
            status: FileStatus::Todo,
            skip_reason: None,
            manual_override: None,
            chunks: vec![ChunkProgress {
                chunk: Chunk {
                    relative_path: path.into(),
                    start_line: 1,
                    end_line: 1,
                    normalized: body.into(),
                    hash: path.into(),
                },
                completion: ChunkCompletion::Incomplete,
                checkpoint: None,
                id: Some(1),
            }],
        };
        RepoProgress {
            files: vec![
                file("src/a.rs", "a"),
                file("src/b.rs", "b"),
                file("lib.rs", "l"),
            ],
        }
    }

    #[test]
    fn filter_keeps_matching_file_and_parent() {
        let progress = progress();
        let mut tree = TreeView::from_progress("demo", &progress, false);
        tree.filter = "b.rs".into();
        tree.refresh_rows(&progress);
        let names: Vec<_> = tree.rows.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"src"));
        assert!(names.contains(&"b.rs"));
        assert!(!names.contains(&"a.rs"));
        assert!(!names.contains(&"lib.rs"));
    }

    #[test]
    fn jump_recommend_selects_first_todo() {
        let progress = progress();
        let mut tree = TreeView::from_progress("demo", &progress, false);
        tree.selected = tree.rows.len() - 1;
        assert!(tree.jump_recommend());
        assert_eq!(tree.selected_file_path().as_deref(), Some("src/a.rs"));
    }

    #[test]
    fn vertical_movement_selects_only_enabled_files() {
        let mut progress = progress();
        progress.files[1].manual_override = Some(crate::domain::content::ManualOverride::Skip);
        let mut tree = TreeView::from_progress("demo", &progress, false);

        assert!(tree.jump_to_path("src/a.rs"));
        tree.move_by(-1);
        assert_eq!(tree.selected_file_path().as_deref(), Some("lib.rs"));

        tree.move_by(1);
        assert_eq!(tree.selected_file_path().as_deref(), Some("src/a.rs"));

        tree.move_by(1);
        assert_eq!(tree.selected_file_path().as_deref(), Some("lib.rs"));
    }
}
