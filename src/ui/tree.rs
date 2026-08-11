//! File tree view model and rendering.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::domain::content::{FileStatus, RepoProgress};
use crate::domain::progress::directory_progress;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRowKind {
    Dir { path: String },
    File { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub kind: TreeRowKind,
    pub depth: usize,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct TreeView {
    pub rows: Vec<TreeRow>,
    pub selected: usize,
    pub recommend: Option<String>,
    pub title: String,
    pub collapsed: std::collections::HashSet<String>,
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
        }
    }

    pub fn refresh_rows(&mut self, progress: &RepoProgress) {
        self.recommend = progress.recommend_path().map(str::to_string);
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
                    let label = format!(
                        "{}/  [{}/{} lines]",
                        part, dprog.completed_lines, dprog.total_lines
                    );
                    rows.push(TreeRow {
                        kind: TreeRowKind::Dir { path: acc.clone() },
                        depth: i,
                        label,
                    });
                    emitted_dirs.insert(acc.clone());
                }
                if collapsed.contains(&acc) {
                    hidden = true;
                }
            } else if !hidden {
                let status = f.derive_status();
                let mark = match status {
                    FileStatus::Done => "✓",
                    FileStatus::Skipped => "·",
                    FileStatus::Todo => "○",
                };
                let mut label = format!("{mark} {part}");
                if status == FileStatus::Skipped {
                    if let Some(reason) = &f.skip_reason {
                        label.push_str(&format!("  ({})", reason.as_str()));
                    }
                } else {
                    label.push_str(&format!("  [{}/{}]", f.completed_lines(), f.total_lines()));
                }
                rows.push(TreeRow {
                    kind: TreeRowKind::File {
                        path: f.relative_path.clone(),
                    },
                    depth: i,
                    label,
                });
            }
        }
    }
    rows
}

pub fn draw_tree(frame: &mut Frame, area: Rect, view: &TreeView) {
    let block = Block::default()
        .title(format!(" {} ", view.title))
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items: Vec<ListItem> = view
        .rows
        .iter()
        .enumerate()
        .map(|(idx, row)| {
            let indent = "  ".repeat(row.depth);
            let mut style = Style::default();
            let mut text = format!("{indent}{}", row.label);
            if let TreeRowKind::File { path } = &row.kind {
                if view.recommend.as_deref() == Some(path.as_str()) {
                    text.push_str("  ← recommend");
                    style = style.fg(Color::Cyan);
                }
            }
            if idx == view.selected {
                style = style.add_modifier(Modifier::REVERSED);
            }
            ListItem::new(Line::from(Span::styled(text, style)))
        })
        .collect();

    let mut state = ListState::default();
    if !view.rows.is_empty() {
        state.select(Some(view.selected));
    }
    let list = List::new(items);
    frame.render_stateful_widget(list, inner, &mut state);

    let help = Paragraph::new("j/k move  Enter open  Space fold  q quit  Esc back");
    let help_area = Rect {
        x: area.x + 1,
        y: area.y.saturating_add(area.height.saturating_sub(1)),
        width: area.width.saturating_sub(2),
        height: 1,
    };
    if help_area.y > area.y {
        frame.render_widget(help, help_area);
    }
}
