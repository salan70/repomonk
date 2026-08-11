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
    /// Repository-wide `(completed, total)` normalized line counts.
    pub overall: (usize, usize),
}

impl TreeView {
    pub fn from_progress(repo_name: &str, progress: &RepoProgress) -> Self {
        let recommend = progress.recommend_path().map(str::to_string);
        let rows = flatten(progress, &std::collections::HashSet::new());
        Self {
            rows,
            selected: 0,
            recommend,
            title: repo_name.to_string(),
            collapsed: std::collections::HashSet::new(),
            overall: overall_progress(progress),
        }
    }

    pub fn refresh_rows(&mut self, progress: &RepoProgress) {
        self.recommend = progress.recommend_path().map(str::to_string);
        self.overall = overall_progress(progress);
        let prev_path = self.selected_file_path();
        self.rows = flatten(progress, &self.collapsed);
        if let Some(path) = prev_path {
            if let Some(idx) = self
                .rows
                .iter()
                .position(|r| matches!(&r.kind, TreeRowKind::File { path: p } if p == &path))
            {
                self.selected = idx;
            } else {
                self.selected = self.selected.min(self.rows.len().saturating_sub(1));
            }
        } else {
            self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len() as isize;
        let next = (self.selected as isize + delta).rem_euclid(len);
        self.selected = next as usize;
    }

    pub fn selected_file_path(&self) -> Option<String> {
        self.rows.get(self.selected).and_then(|r| match &r.kind {
            TreeRowKind::File { path } => Some(path.clone()),
            TreeRowKind::Dir { .. } => None,
        })
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

fn flatten(progress: &RepoProgress, collapsed: &std::collections::HashSet<String>) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    // Build a simple path-sorted expansion.
    let mut dirs: Vec<String> = Vec::new();
    for f in &progress.files {
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
                    (None, f.skip_reason.as_ref().map(|r| r.as_str().to_string()))
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
    rows
}

pub fn draw_tree(frame: &mut Frame, area: Rect, view: &TreeView) {
    theme::fill_background(frame, area);
    let block = theme::bordered_block(theme::title_line(&view.title));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

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
            format!("{completed}/{total} lines · {:.0}%", ratio * 100.0),
            Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
        ));
    let gauge_area = Rect {
        x: panes[0].x + 1,
        y: panes[0].y,
        width: panes[0].width.saturating_sub(2),
        height: panes[0].height,
    };
    frame.render_widget(gauge, gauge_area);

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
    frame.render_stateful_widget(list, panes[2], &mut state);

    // Footer: key hints.
    frame.render_widget(
        Paragraph::new(theme::key_hints(&[
            ("j/k", "move"),
            ("Enter", "open"),
            ("Space", "fold"),
            ("q/Esc", "quit"),
        ])),
        panes[3],
    );
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
