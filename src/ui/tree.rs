//! File tree view model and rendering.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::domain::content::{FileStatus, ManualOverride, RepoProgress, SkipReason};
use crate::domain::dependency::FlowOrder;
use crate::domain::progress::directory_progress;
use crate::ui::i18n::{display_width, UiStrings};
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
    /// For directories: `(done_files, typeable_files)`. `None` for files.
    pub file_counts: Option<(usize, usize)>,
    /// File status; `None` for directories.
    pub status: Option<FileStatus>,
    /// Automatic skip reason for skipped files (persistence key, not display).
    pub skip_reason: Option<SkipReason>,
    /// True when the user skipped this file with `x`.
    pub manual_skip: bool,
}

#[derive(Debug, Clone)]
pub struct TreeView {
    pub rows: Vec<TreeRow>,
    pub selected: usize,
    pub recommend: Option<String>,
    pub title: String,
    /// Directories the user has opened. Anything absent is closed, so a directory
    /// that only appears once excluded files are shown starts closed.
    pub opened: std::collections::HashSet<String>,
    /// Flow traversal order, when flow mode is active.
    pub flow: Option<FlowOrder>,
    /// Reachable `(done, total)` file counts for the flow bar.
    pub flow_counts: Option<(usize, usize)>,
    /// Repository-wide `(completed, total)` normalized line counts.
    pub overall: (usize, usize),
    /// Repository-wide `(done_files, typeable_files)`.
    pub file_counts: (usize, usize),
    /// Line count of the recommended file, for the Next row.
    pub recommend_lines: Option<usize>,
    pub hide_skipped: bool,
    /// Incremental file filter (`/` on Tree).
    pub filter: String,
    pub filter_editing: bool,
    /// Last drawn list height, used for half-page jumps.
    pub visible_rows: usize,
    /// Index of the first row drawn. Owned here rather than left to the list widget
    /// so rebuilding the tree keeps the cursor on the same screen line.
    pub offset: usize,
    /// Shown after Result Enter when the repository is fully done.
    pub repo_complete: bool,
    /// Count of skipped files, whether or not `hide_skipped` is currently hiding them.
    pub excluded: usize,
    /// One-line status/error banner (e.g. from the File types overlay), cleared on next input.
    pub message: Option<String>,
    /// Paths whose file type is set to `Hidden`: excluded from the tree unconditionally,
    /// regardless of `hide_skipped`. Rolls up to hide a directory whose files are all hidden.
    pub hidden_paths: std::collections::HashSet<String>,
    /// True when every directory in the (unfiltered) tree is in `opened`.
    pub fully_expanded: bool,
}

impl TreeView {
    pub fn from_progress_full(
        repo_name: &str,
        progress: &RepoProgress,
        hide_skipped: bool,
        flow: Option<FlowOrder>,
        hidden_paths: std::collections::HashSet<String>,
    ) -> Self {
        let recommend = recommended_path(progress, flow.as_ref());
        let recommend_lines = recommend.as_ref().and_then(|path| {
            progress
                .files
                .iter()
                .find(|f| f.relative_path == *path)
                .map(|f| f.total_lines())
        });
        let flow_counts = flow
            .as_ref()
            .map(|order| (order.reachable_done(progress), order.reachable_total()));
        let root = directory_progress(progress, "");
        let all_rows = compress_chains(flatten(progress, hide_skipped, &hidden_paths));
        let opened: std::collections::HashSet<String> = all_rows
            .iter()
            .filter_map(|row| match &row.kind {
                TreeRowKind::Dir { path } => Some(path.clone()),
                TreeRowKind::File { .. } => None,
            })
            .collect();
        let fully_expanded = !opened.is_empty();
        let rows = apply_collapse(all_rows, &opened);
        let mut view = Self {
            rows,
            selected: 0,
            recommend,
            title: repo_name.to_string(),
            opened,
            flow,
            flow_counts,
            overall: (root.completed_lines, root.total_lines),
            file_counts: (root.done_files, root.done_files + root.todo_files),
            recommend_lines,
            hide_skipped,
            filter: String::new(),
            filter_editing: false,
            visible_rows: 1,
            offset: 0,
            repo_complete: false,
            excluded: excluded_count(progress, &hidden_paths),
            message: None,
            hidden_paths,
            fully_expanded,
        };
        view.jump_recommend();
        // The real list height is unknown until the first draw, so leave the scroll
        // at the top and let `set_visible_rows` scroll only as far as it must.
        view.offset = 0;
        view
    }

    /// Called with the drawn list height before each frame.
    pub fn set_visible_rows(&mut self, height: usize) {
        self.visible_rows = height;
        self.clamp_offset();
    }

    pub fn refresh_rows(&mut self, progress: &RepoProgress) {
        self.recommend = recommended_path(progress, self.flow.as_ref());
        self.recommend_lines = self.recommend.as_ref().and_then(|path| {
            progress
                .files
                .iter()
                .find(|f| f.relative_path == *path)
                .map(|f| f.total_lines())
        });
        self.flow_counts = self
            .flow
            .as_ref()
            .map(|order| (order.reachable_done(progress), order.reachable_total()));
        let root = directory_progress(progress, "");
        self.overall = (root.completed_lines, root.total_lines);
        self.file_counts = (root.done_files, root.done_files + root.todo_files);
        self.excluded = excluded_count(progress, &self.hidden_paths);
        let prev_paths: Vec<String> = self.rows.iter().map(row_path).collect();
        let prev_selected = self.selected;
        let screen_row = prev_selected.saturating_sub(self.offset);
        self.fully_expanded = is_fully_expanded(
            &self.opened,
            progress,
            self.hide_skipped,
            &self.hidden_paths,
        );
        self.rows = visible_rows(
            progress,
            &self.opened,
            self.hide_skipped,
            &self.hidden_paths,
            &self.filter,
        );
        self.selected = self
            .surviving_row(&prev_paths, prev_selected)
            .unwrap_or_else(|| self.selected.min(self.rows.len().saturating_sub(1)));
        // Scroll so the cursor stays on the screen line it was already on.
        self.offset = self.selected.saturating_sub(screen_row);
        self.clamp_offset();
    }

    fn select_index(&mut self, index: usize) {
        self.selected = index;
        self.clamp_offset();
    }

    /// Scroll only as far as it takes to keep the cursor inside the drawn window.
    ///
    /// The offset is deliberately not capped at `rows.len() - height`: revealing
    /// excluded files must not push the cursor down the screen just because the
    /// shorter list happened to fit. Trailing blank lines are the lesser evil.
    fn clamp_offset(&mut self) {
        let height = self.visible_rows.max(1);
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + height {
            self.offset = self.selected + 1 - height;
        }
    }

    /// Keep the cursor on the row it was on. When that row is gone (e.g. an excluded
    /// file after `.` re-hides it), fall back to the nearest row that survived,
    /// searching upwards first so the cursor settles on the enclosing directory.
    fn surviving_row(&self, prev_paths: &[String], prev_selected: usize) -> Option<usize> {
        let index_of = |path: &String| self.rows.iter().position(|r| &row_path(r) == path);
        let before = (0..=prev_selected).rev();
        let after = (prev_selected + 1)..prev_paths.len();
        before
            .chain(after)
            .filter_map(|i| prev_paths.get(i))
            .find_map(index_of)
    }

    pub fn set_flow(&mut self, progress: &RepoProgress, flow: Option<FlowOrder>) {
        self.flow = flow;
        self.refresh_rows(progress);
    }

    pub fn flow_step_number(&self, path: &str) -> Option<usize> {
        self.flow.as_ref().and_then(|order| order.step_number(path))
    }

    /// Flip between hiding and showing excluded files for this session only;
    /// `progress.hide_skipped` stays the startup default.
    pub fn toggle_hide_skipped(&mut self, progress: &RepoProgress) {
        self.hide_skipped = !self.hide_skipped;
        self.refresh_rows(progress);
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() || delta == 0 {
            return;
        }
        let len = self.rows.len() as isize;
        self.select_index((self.selected as isize + delta).rem_euclid(len) as usize);
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
        self.select_index(0);
    }

    pub fn select_last(&mut self) {
        self.select_index(self.rows.len().saturating_sub(1));
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
            self.select_index(idx);
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
                self.select_index(idx);
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
            if self.opened.insert(path) {
                self.refresh_rows(progress);
            }
        }
    }

    pub fn collapse_or_parent(&mut self, progress: &RepoProgress) {
        match self.rows.get(self.selected).map(|r| r.kind.clone()) {
            Some(TreeRowKind::Dir { path }) => {
                if self.opened.remove(&path) {
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
        let mut current = Some(path.to_string());
        while let Some(p) = current {
            if let Some(idx) = self
                .rows
                .iter()
                .position(|r| matches!(&r.kind, TreeRowKind::Dir { path: dp } if dp == &p))
            {
                self.select_index(idx);
                return;
            }
            current = parent_path(&p);
        }
    }

    pub fn toggle_collapse(&mut self, progress: &RepoProgress) {
        if let Some(TreeRow {
            kind: TreeRowKind::Dir { path },
            ..
        }) = self.rows.get(self.selected)
        {
            let path = path.clone();
            if !self.opened.remove(&path) {
                self.opened.insert(path);
            }
            self.refresh_rows(progress);
        }
    }

    /// Expand every directory, or if already fully expanded, collapse all except
    /// the ancestors of the selected row so the cursor stays visible.
    pub fn toggle_expand_all(&mut self, progress: &RepoProgress) {
        let dirs = all_dir_paths(progress, self.hide_skipped, &self.hidden_paths);
        if dirs.is_empty() {
            return;
        }
        if dirs.iter().all(|path| self.opened.contains(path)) {
            let keep = self
                .selected_path()
                .or_else(|| self.recommend.clone())
                .unwrap_or_default();
            self.opened.clear();
            open_ancestors(&mut self.opened, &keep);
        } else {
            self.opened.extend(dirs);
        }
        self.refresh_rows(progress);
    }
}

fn excluded_count(
    progress: &RepoProgress,
    hidden_paths: &std::collections::HashSet<String>,
) -> usize {
    progress
        .files
        .iter()
        .filter(|f| {
            f.derive_status() == FileStatus::Skipped && !hidden_paths.contains(&f.relative_path)
        })
        .count()
}

fn header_label(view: &TreeView, t: &UiStrings) -> String {
    let (completed, total) = view.overall;
    let ratio = if total == 0 {
        0.0
    } else {
        completed as f64 / total as f64
    };
    let pct = format!("{:.0}%", ratio * 100.0);
    if view.flow.is_some() {
        let (done, files) = view.flow_counts.unwrap_or(view.file_counts);
        format!("{pct}  ·  {done}/{files} {}", t.files_in_flow)
    } else {
        let (done, files) = view.file_counts;
        format!("{pct}  ·  {done}/{files} {}", t.files)
    }
}

fn next_line(view: &TreeView, t: &UiStrings) -> Line<'static> {
    match &view.recommend {
        Some(path) => {
            let text = if view.flow.is_some() {
                if let Some(step) = view.flow_step_number(path) {
                    format!(
                        " ▸ {}  {} {step} · {path} · {}",
                        t.next_label, t.next_step, t.enter_to_start
                    )
                } else {
                    format!(" ▸ {}  {path} · {}", t.next_label, t.enter_to_start)
                }
            } else {
                let lines = view.recommend_lines.unwrap_or(0);
                format!(
                    " ▸ {}  {path} · {lines} {} · {}",
                    t.next_label, t.lines, t.enter_to_start
                )
            };
            Line::from(Span::styled(
                text,
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD),
            ))
        }
        None if view.repo_complete => Line::from(Span::styled(
            t.repo_complete.to_string(),
            Style::default().fg(theme::GREEN),
        )),
        None => Line::from(""),
    }
}

fn recommended_path(progress: &RepoProgress, flow: Option<&FlowOrder>) -> Option<String> {
    if let Some(order) = flow {
        return order.next_step(progress).map(|step| step.path.clone());
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

fn open_ancestors(opened: &mut std::collections::HashSet<String>, path: &str) {
    let mut current = path.to_string();
    while let Some(parent) = parent_path(&current) {
        opened.insert(parent.clone());
        current = parent;
    }
}

fn all_dir_paths(
    progress: &RepoProgress,
    hide_skipped: bool,
    hidden_paths: &std::collections::HashSet<String>,
) -> std::collections::HashSet<String> {
    compress_chains(flatten(progress, hide_skipped, hidden_paths))
        .into_iter()
        .filter_map(|row| match row.kind {
            TreeRowKind::Dir { path } => Some(path),
            TreeRowKind::File { .. } => None,
        })
        .collect()
}

fn is_fully_expanded(
    opened: &std::collections::HashSet<String>,
    progress: &RepoProgress,
    hide_skipped: bool,
    hidden_paths: &std::collections::HashSet<String>,
) -> bool {
    let dirs = all_dir_paths(progress, hide_skipped, hidden_paths);
    !dirs.is_empty() && dirs.iter().all(|path| opened.contains(path))
}

fn visible_rows(
    progress: &RepoProgress,
    opened: &std::collections::HashSet<String>,
    hide_skipped: bool,
    hidden_paths: &std::collections::HashSet<String>,
    filter: &str,
) -> Vec<TreeRow> {
    let rows = flatten(progress, hide_skipped, hidden_paths);
    let rows = compress_chains(rows);
    let rows = apply_collapse(rows, opened);
    filter_rows(rows, filter)
}

fn flatten(
    progress: &RepoProgress,
    hide_skipped: bool,
    hidden_paths: &std::collections::HashSet<String>,
) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    let mut emitted_dirs = std::collections::HashSet::new();
    for f in &progress.files {
        if hidden_paths.contains(&f.relative_path)
            || (hide_skipped && f.derive_status() == FileStatus::Skipped)
        {
            continue;
        }
        let parts: Vec<&str> = f.relative_path.split('/').collect();
        let mut acc = String::new();
        for (i, part) in parts.iter().enumerate() {
            let is_file = i + 1 == parts.len();
            if !is_file {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(part);
                if !emitted_dirs.contains(&acc) {
                    let dprog = directory_progress(progress, &acc);
                    rows.push(TreeRow {
                        kind: TreeRowKind::Dir { path: acc.clone() },
                        depth: i,
                        name: (*part).to_string(),
                        progress: Some((dprog.completed_lines, dprog.total_lines)),
                        file_counts: Some((dprog.done_files, dprog.done_files + dprog.todo_files)),
                        status: None,
                        skip_reason: None,
                        manual_skip: false,
                    });
                    emitted_dirs.insert(acc.clone());
                }
            } else {
                let status = f.derive_status();
                let (progress_counts, skip_reason, manual_skip) = if status == FileStatus::Skipped {
                    (
                        None,
                        f.skip_reason.clone(),
                        f.manual_override == Some(ManualOverride::Skip),
                    )
                } else {
                    (Some((f.completed_lines(), f.total_lines())), None, false)
                };
                rows.push(TreeRow {
                    kind: TreeRowKind::File {
                        path: f.relative_path.clone(),
                    },
                    depth: i,
                    name: (*part).to_string(),
                    progress: progress_counts,
                    file_counts: None,
                    status: Some(status),
                    skip_reason,
                    manual_skip,
                });
            }
        }
    }
    rows
}

fn compress_chains(mut rows: Vec<TreeRow>) -> Vec<TreeRow> {
    loop {
        let next = compress_once(&rows);
        if next == rows {
            return next;
        }
        rows = next;
    }
}

fn compress_once(rows: &[TreeRow]) -> Vec<TreeRow> {
    let mut out = Vec::with_capacity(rows.len());
    let mut i = 0;
    while i < rows.len() {
        let row = &rows[i];
        if matches!(row.kind, TreeRowKind::Dir { .. }) {
            let mut end = i + 1;
            while end < rows.len() && rows[end].depth > row.depth {
                end += 1;
            }
            let children: Vec<usize> = (i + 1..end)
                .filter(|&k| rows[k].depth == row.depth + 1)
                .collect();
            if children.len() == 1 {
                let child_idx = children[0];
                if let TreeRowKind::Dir { path: child_path } = &rows[child_idx].kind {
                    let mut compressed = row.clone();
                    compressed.name = format!("{}/{}", row.name, rows[child_idx].name);
                    compressed.kind = TreeRowKind::Dir {
                        path: child_path.clone(),
                    };
                    compressed.progress = rows[child_idx].progress;
                    compressed.file_counts = rows[child_idx].file_counts;
                    out.push(compressed);
                    for desc in rows.iter().take(end).skip(child_idx + 1) {
                        let mut lifted = desc.clone();
                        lifted.depth = lifted.depth.saturating_sub(1);
                        out.push(lifted);
                    }
                    i = end;
                    continue;
                }
            }
        }
        out.push(row.clone());
        i += 1;
    }
    out
}

fn apply_collapse(rows: Vec<TreeRow>, opened: &std::collections::HashSet<String>) -> Vec<TreeRow> {
    let mut out = Vec::new();
    let mut hidden_until_depth: Option<usize> = None;
    for row in rows {
        if let Some(depth) = hidden_until_depth {
            if row.depth > depth {
                continue;
            }
            hidden_until_depth = None;
        }
        if let TreeRowKind::Dir { path } = &row.kind {
            if !opened.contains(path) {
                hidden_until_depth = Some(row.depth);
            }
        }
        out.push(row);
    }
    out
}

/// Elide the middle of a compressed directory name when it exceeds `max` cells.
fn elide_dir_name(name: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let char_len = name.chars().count();
    if char_len <= max {
        return name.to_string();
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() >= 3 {
        let candidate = format!("{}/…/{}", parts[0], parts[parts.len() - 1]);
        if candidate.chars().count() <= max {
            return candidate;
        }
    }
    if max <= 1 {
        return "…".to_string();
    }
    let chars: Vec<char> = name.chars().collect();
    let tail: String = chars[chars.len() - (max - 1)..].iter().collect();
    format!("…{tail}")
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

/// Inner list height for a Tree screen of `area`.
///
/// Subtracts 4 chrome rows (header, Next, detail, footer) and 2 border rows.
pub fn list_height(area: Rect) -> usize {
    area.height.saturating_sub(4 + 2) as usize
}

pub fn draw_tree(frame: &mut Frame, area: Rect, view: &TreeView, t: &UiStrings) {
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
            Constraint::Length(1),
        ])
        .split(inner);

    let gauge_pane = panes[0];
    let next_pane = panes[1];
    let list_pane = panes[2];
    let info_pane = panes[3];
    let footer_pane = panes[4];

    // Header: line-based gauge ratio, files-based label.
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
            header_label(view, t),
            Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
        ));
    let gauge_area = Rect {
        x: gauge_pane.x + 1,
        y: gauge_pane.y,
        width: gauge_pane.width.saturating_sub(2),
        height: gauge_pane.height,
    };
    frame.render_widget(gauge, gauge_area);
    frame.render_widget(Paragraph::new(next_line(view, t)), next_pane);

    frame.render_widget(
        Paragraph::new(info_line(view, info_pane.width, t)),
        info_pane,
    );

    // Body: tree rows.
    let items: Vec<ListItem> = view
        .rows
        .iter()
        .map(|row| ListItem::new(row_line(row, view, list_pane.width, t)))
        .collect();

    let mut state = ListState::default().with_offset(view.offset);
    if !view.rows.is_empty() {
        state.select(Some(view.selected));
    }
    let list = List::new(items)
        .highlight_style(Style::default().bg(theme::SELECTION_BG))
        .style(theme::base_style());
    frame.render_stateful_widget(list, list_pane, &mut state);

    frame.render_widget(
        Paragraph::new(theme::key_hints(&tree_footer_hints(view, t))),
        footer_pane,
    );
}

fn tree_footer_hints<'a>(view: &TreeView, t: &'a UiStrings) -> Vec<(&'static str, &'a str)> {
    if view.filter_editing {
        vec![
            ("Esc", t.clear),
            ("n/N", t.next),
            ("j/k", t.move_),
            ("Enter", t.open),
            ("?", t.more),
        ]
    } else {
        let expand = if view.fully_expanded {
            t.collapse_all
        } else {
            t.expand_all
        };
        vec![
            ("Enter", t.open),
            ("j/k", t.move_),
            ("h/l", t.fold),
            ("o", expand),
            ("Tab", t.next_file),
            ("?", t.more),
        ]
    }
}

fn info_line(view: &TreeView, width: u16, t: &UiStrings) -> Line<'static> {
    if view.filter_editing || !view.filter.is_empty() {
        return Line::from(vec![
            Span::styled(" /", Style::default().fg(theme::CYAN)),
            Span::styled(
                view.filter.clone(),
                Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if view.filter_editing { "▌" } else { "" },
                Style::default().fg(theme::CYAN),
            ),
        ]);
    }
    if let Some(message) = &view.message {
        return Line::from(Span::styled(
            format!(" {message}"),
            Style::default().fg(theme::MUTED),
        ));
    }
    let left = selected_detail(view, t);
    let right = if view.excluded == 0 {
        String::new()
    } else if view.hide_skipped {
        format!("{} {} · .", view.excluded, t.excluded)
    } else {
        format!("{} {} · .", view.excluded, t.excluded_shown)
    };
    let pad = (width as usize).saturating_sub(display_width(&left) + display_width(&right) + 2);
    Line::from(vec![
        Span::styled(format!(" {left}"), Style::default().fg(theme::MUTED)),
        Span::raw(" ".repeat(pad)),
        Span::styled(right, Style::default().fg(theme::MUTED)),
    ])
}

fn selected_detail(view: &TreeView, t: &UiStrings) -> String {
    let Some(row) = view.rows.get(view.selected) else {
        return String::new();
    };
    match &row.kind {
        TreeRowKind::File { path } => {
            let mut text = if row.manual_skip || row.skip_reason.is_some() {
                format!(
                    "{path} · {}",
                    t.skip_full_opt(row.skip_reason.as_ref(), row.manual_skip)
                )
            } else if let Some((done, total)) = row.progress {
                format!("{path} · {total} {} · {done} {}", t.lines, t.done)
            } else {
                path.clone()
            };
            if let Some(origin) = selected_origin(view, t) {
                text.push_str(" · ");
                text.push_str(&origin);
            }
            text
        }
        TreeRowKind::Dir { path } => {
            let (done, total) = row.file_counts.unwrap_or((0, 0));
            let mut text = format!("{path}/ · {total} {} · {done} {}", t.files, t.done);
            if let Some(flow) = &view.flow {
                text.push_str(" · ");
                text.push_str(t.entry);
                text.push(' ');
                text.push_str(&flow.entry);
            }
            text
        }
    }
}

fn mini_bar(ratio: f64, width: usize) -> String {
    let filled = ((ratio.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    let mut s = String::new();
    for i in 0..width {
        if i < filled {
            s.push('█');
        } else {
            s.push('░');
        }
    }
    s
}

fn selected_origin(view: &TreeView, t: &UiStrings) -> Option<String> {
    let flow = view.flow.as_ref()?;
    let path = view.selected_file_path()?;
    if path == flow.entry {
        return Some(t.entry_point.to_string());
    }
    if let Some(via) = flow.via(&path) {
        return Some(format!("← {}:{}  {}", via.importer, via.line, via.raw));
    }
    match flow.is_reachable(&path) {
        Some(true) => Some(t.entry_point.to_string()),
        Some(false) | None => Some(t.outside_flow.to_string()),
    }
}

fn row_line(row: &TreeRow, view: &TreeView, width: u16, t: &UiStrings) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();
    let indent = 1 + row.depth * 2;
    spans.push(Span::raw(" ".repeat(indent)));

    match &row.kind {
        TreeRowKind::Dir { path } => {
            let arrow = if view.opened.contains(path) {
                "▾ "
            } else {
                "▸ "
            };
            spans.push(Span::styled(
                arrow.to_string(),
                Style::default().fg(theme::MUTED),
            ));
            let bar = match row.progress {
                Some((done, total)) if total > 0 => mini_bar(done as f64 / total as f64, 8),
                Some(_) => mini_bar(0.0, 8),
                None => String::new(),
            };
            let used = indent + arrow.chars().count() + 1 + bar.chars().count() + 1;
            let budget = (width as usize).saturating_sub(used);
            let name = elide_dir_name(&row.name, budget);
            let name_text = format!("{name}/");
            spans.push(Span::styled(
                name_text.clone(),
                Style::default()
                    .fg(theme::BLUE)
                    .add_modifier(Modifier::BOLD),
            ));
            if !bar.is_empty() {
                let pad = (width as usize).saturating_sub(
                    indent
                        + arrow.chars().count()
                        + name_text.chars().count()
                        + bar.chars().count(),
                );
                spans.push(Span::raw(" ".repeat(pad.max(1))));
                spans.push(Span::styled(bar, Style::default().fg(theme::MUTED)));
            }
        }
        TreeRowKind::File { path } => {
            if view.flow.is_some() {
                if let Some(number) = view.flow_step_number(path) {
                    let status = row.status.unwrap_or(FileStatus::Todo);
                    let color = if status == FileStatus::Done {
                        theme::GREEN
                    } else {
                        theme::CYAN
                    };
                    spans.push(Span::styled(
                        format!("{number:>3} "),
                        Style::default().fg(color),
                    ));
                } else {
                    spans.push(Span::styled(
                        "  — ".to_string(),
                        Style::default().fg(theme::MUTED),
                    ));
                }
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
            if row.manual_skip || row.skip_reason.is_some() {
                let short = t.skip_short(row.skip_reason.as_ref(), row.manual_skip);
                spans.push(Span::styled(
                    format!(" — {short}"),
                    Style::default()
                        .fg(theme::MUTED)
                        .add_modifier(Modifier::ITALIC),
                ));
            }
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::UiLanguage;
    use crate::domain::content::{
        Chunk, ChunkCompletion, ChunkProgress, FileProgress, FileStatus, RepoProgress, SkipReason,
    };
    use crate::ui::i18n::strings;

    fn en() -> &'static crate::ui::i18n::UiStrings {
        strings(UiLanguage::En)
    }

    fn file(path: &str, body: &str) -> FileProgress {
        FileProgress {
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
        }
    }

    fn progress() -> RepoProgress {
        RepoProgress {
            files: vec![
                file("src/a.rs", "a"),
                file("src/b.rs", "b"),
                file("lib.rs", "l"),
            ],
        }
    }

    fn tree_view(progress: &RepoProgress, hide_skipped: bool) -> TreeView {
        TreeView::from_progress_full(
            "demo",
            progress,
            hide_skipped,
            None,
            std::collections::HashSet::new(),
        )
    }

    fn skipped_file(path: &str, reason: SkipReason) -> FileProgress {
        FileProgress {
            relative_path: path.into(),
            status: FileStatus::Skipped,
            skip_reason: Some(reason),
            manual_override: None,
            chunks: vec![],
        }
    }

    #[test]
    fn hide_skipped_default_removes_skipped_rows_and_empty_dirs() {
        let mut progress = progress();
        progress
            .files
            .push(skipped_file("tests/foo.rs", SkipReason::TestFile));
        progress
            .files
            .push(skipped_file("docs/readme.md", SkipReason::ConfigFile));
        let tree = tree_view(&progress, true);
        let names: Vec<_> = tree.rows.iter().map(|r| r.name.as_str()).collect();
        assert!(!names.contains(&"tests"));
        assert!(!names.contains(&"foo.rs"));
        assert!(!names.contains(&"docs"));
        assert!(!names.contains(&"readme.md"));
        assert!(names.contains(&"src"));
        assert!(names.contains(&"a.rs"));
        assert!(names.contains(&"lib.rs"));
        assert_eq!(tree.excluded, 2);
    }

    #[test]
    fn filter_keeps_matching_file_and_parent() {
        let progress = progress();
        let mut tree = tree_view(&progress, false);
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
        let mut tree = tree_view(&progress, false);
        tree.selected = tree.rows.len() - 1;
        assert!(tree.jump_recommend());
        assert_eq!(tree.selected_file_path().as_deref(), Some("src/a.rs"));
    }

    #[test]
    fn vertical_movement_stops_on_excluded_rows_while_shown() {
        let mut progress = progress();
        progress.files[1].manual_override = Some(crate::domain::content::ManualOverride::Skip);
        let mut tree = tree_view(&progress, false);

        assert!(tree.jump_to_path("src/a.rs"));
        tree.move_by(-1);
        assert_eq!(tree.selected_dir_path().as_deref(), Some("src"));

        tree.move_by(-1);
        assert_eq!(tree.selected_file_path().as_deref(), Some("lib.rs"));

        tree.move_by(1);
        assert_eq!(tree.selected_dir_path().as_deref(), Some("src"));

        tree.move_by(1);
        assert_eq!(tree.selected_file_path().as_deref(), Some("src/a.rs"));

        // The excluded file is reachable so `x` can rescue it.
        tree.move_by(1);
        assert_eq!(tree.selected_file_path().as_deref(), Some("src/b.rs"));

        tree.move_by(1);
        assert_eq!(tree.selected_file_path().as_deref(), Some("lib.rs"));
    }

    #[test]
    fn toggle_hide_skipped_reveals_and_rehides_excluded_rows() {
        let mut progress = progress();
        progress
            .files
            .push(skipped_file("tests/foo.rs", SkipReason::TestFile));
        let mut tree = tree_view(&progress, true);
        assert!(tree.hide_skipped);
        let hidden: Vec<_> = tree.rows.iter().map(|r| r.name.as_str()).collect();
        assert!(!hidden.contains(&"tests"));

        tree.toggle_hide_skipped(&progress);
        assert!(!tree.hide_skipped);
        let shown: Vec<_> = tree.rows.iter().map(|r| r.name.as_str()).collect();
        assert!(shown.contains(&"tests"));
        // `tests/` appears for the first time here, so it comes in closed like every
        // other directory does at open, rather than dumping its contents on screen.
        assert!(!shown.contains(&"foo.rs"));
        tree.opened.insert("tests".into());
        tree.refresh_rows(&progress);
        let names: Vec<_> = tree.rows.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"foo.rs"));

        tree.opened.remove("tests");
        tree.toggle_hide_skipped(&progress);
        assert!(tree.hide_skipped);
        let names: Vec<_> = tree.rows.iter().map(|r| r.name.as_str()).collect();
        assert!(!names.contains(&"tests"));
        assert!(!names.contains(&"foo.rs"));
    }

    #[test]
    fn toggling_keeps_the_cursor_on_the_selected_row() {
        let mut progress = progress();
        progress
            .files
            .push(skipped_file("tests/foo.rs", SkipReason::TestFile));
        let mut tree = tree_view(&progress, true);
        assert!(tree.jump_to_path("src/b.rs"));

        tree.toggle_hide_skipped(&progress);
        assert_eq!(tree.selected_path().as_deref(), Some("src/b.rs"));
        tree.toggle_hide_skipped(&progress);
        assert_eq!(tree.selected_path().as_deref(), Some("src/b.rs"));
    }

    /// Excluded files sit above the row under the cursor, so revealing them pushes
    /// that row far down the list. Without an owned offset the list widget would
    /// scroll it to the bottom edge instead of leaving it where it was.
    fn progress_with_excluded_above() -> RepoProgress {
        let mut files: Vec<FileProgress> = (0..5)
            .map(|i| file(&format!("src/a{i:02}.rs"), "a"))
            .collect();
        files.extend(
            (0..30).map(|i| skipped_file(&format!("src/z{i:02}.rs"), SkipReason::TestFile)),
        );
        files.extend((0..30).map(|i| file(&format!("src/b{i:02}.rs"), "b")));
        RepoProgress { files }
    }

    #[test]
    fn toggling_keeps_the_cursor_on_the_same_screen_line() {
        let progress = progress_with_excluded_above();
        let mut tree = tree_view(&progress, true);
        tree.visible_rows = 14;
        assert!(tree.jump_to_path("src/b02.rs"));

        let screen_line = tree.selected - tree.offset;
        assert!(screen_line < tree.visible_rows);

        tree.toggle_hide_skipped(&progress);
        assert_eq!(tree.selected_path().as_deref(), Some("src/b02.rs"));
        assert_eq!(tree.selected - tree.offset, screen_line);

        tree.toggle_hide_skipped(&progress);
        assert_eq!(tree.selected_path().as_deref(), Some("src/b02.rs"));
        assert_eq!(tree.selected - tree.offset, screen_line);
    }

    /// Screen line (within the list pane) that the cursor is drawn on, read back
    /// from a real render so the assertion covers the list widget's own scrolling.
    fn drawn_cursor_line(view: &TreeView, width: u16, height: u16) -> Option<usize> {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw_tree(frame, frame.area(), view, en()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let selected = view.rows.get(view.selected)?;
        let list_top = 3; // border + gauge + Next
        (0..list_height(Rect::new(0, 0, width, height))).find(|i| {
            let y = (list_top + i) as u16;
            let line: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
            line.contains(&selected.name)
        })
    }

    #[test]
    fn revealing_does_not_push_the_cursor_down_when_the_hidden_list_fit() {
        // Hidden, this tree is short enough to fit with room to spare; shown, it is
        // far taller. The cursor must hold its line rather than be pushed down by
        // the rows inserted above it.
        let mut files: Vec<FileProgress> = (0..5)
            .map(|i| file(&format!("src/a{i:02}.rs"), "a"))
            .collect();
        files.extend(
            (0..30).map(|i| skipped_file(&format!("src/z{i:02}.rs"), SkipReason::TestFile)),
        );
        files.extend((0..5).map(|i| file(&format!("src/b{i:02}.rs"), "b")));
        let progress = RepoProgress { files };

        let mut tree = tree_view(&progress, true);
        tree.set_visible_rows(20);
        assert!(
            tree.rows.len() < tree.visible_rows,
            "hidden list should fit"
        );
        assert!(tree.jump_to_path("src/b02.rs"));
        let screen_line = tree.selected - tree.offset;

        tree.toggle_hide_skipped(&progress);
        assert!(
            tree.rows.len() > tree.visible_rows,
            "shown list should overflow"
        );
        assert_eq!(tree.selected_path().as_deref(), Some("src/b02.rs"));
        assert_eq!(tree.selected - tree.offset, screen_line);

        tree.toggle_hide_skipped(&progress);
        assert_eq!(tree.selected - tree.offset, screen_line);
    }

    #[test]
    fn opening_does_not_pin_the_cursor_to_the_top() {
        let progress = progress_with_excluded_above();
        let (width, height) = (60, 20);

        // `src/a00.rs` is the recommended file, so it sits at its natural depth in
        // the list. Opening must not scroll it up to the first line.
        let mut tree = tree_view(&progress, true);
        tree.set_visible_rows(list_height(Rect::new(0, 0, width, height)));

        assert_eq!(tree.selected_path().as_deref(), Some("src/a00.rs"));
        assert_eq!(tree.offset, 0);
        assert_eq!(drawn_cursor_line(&tree, width, height), Some(tree.selected));
    }

    #[test]
    fn toggling_draws_the_cursor_on_the_same_screen_line() {
        let progress = progress_with_excluded_above();
        let (width, height) = (60, 20);

        let mut tree = tree_view(&progress, true);
        tree.visible_rows = list_height(Rect::new(0, 0, width, height));
        assert!(tree.jump_to_path("src/b02.rs"));

        let before = drawn_cursor_line(&tree, width, height);
        assert!(before.is_some(), "cursor row should be on screen");

        tree.toggle_hide_skipped(&progress);
        assert_eq!(drawn_cursor_line(&tree, width, height), before);

        tree.toggle_hide_skipped(&progress);
        assert_eq!(drawn_cursor_line(&tree, width, height), before);
    }

    #[test]
    fn rehiding_the_selected_excluded_row_falls_back_to_the_nearest_row_above() {
        let mut progress = progress();
        progress
            .files
            .push(skipped_file("tests/foo.rs", SkipReason::TestFile));
        let mut tree = tree_view(&progress, false);
        tree.opened.insert("tests".into());
        tree.refresh_rows(&progress);
        assert!(tree.jump_to_path("tests/foo.rs"));

        // `tests/` holds nothing else, so both it and the file disappear.
        tree.toggle_hide_skipped(&progress);
        assert_eq!(tree.selected_path().as_deref(), Some("lib.rs"));
    }

    #[test]
    fn info_line_labels_both_excluded_modes() {
        let mut progress = progress();
        progress
            .files
            .push(skipped_file("tests/foo.rs", SkipReason::TestFile));
        let mut tree = tree_view(&progress, true);

        let hidden: String = info_line(&tree, 100, en())
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(hidden.contains("1 excluded · ."), "{hidden}");

        tree.toggle_hide_skipped(&progress);
        let shown: String = info_line(&tree, 100, en())
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(shown.contains("1 excluded shown · ."), "{shown}");
    }

    #[test]
    fn excluded_count_is_independent_of_hide_skipped() {
        let mut progress = progress();
        progress
            .files
            .push(skipped_file("tests/foo.rs", SkipReason::TestFile));
        let mut tree = tree_view(&progress, true);
        assert_eq!(tree.excluded, 1);
        tree.toggle_hide_skipped(&progress);
        assert_eq!(tree.excluded, 1);
    }

    #[test]
    fn move_by_stops_on_directories() {
        let progress = progress();
        let mut tree = tree_view(&progress, false);
        assert_eq!(tree.selected_file_path().as_deref(), Some("src/a.rs"));
        tree.move_by(-1);
        assert_eq!(tree.selected_dir_path().as_deref(), Some("src"));
        tree.move_by(1);
        assert_eq!(tree.selected_file_path().as_deref(), Some("src/a.rs"));
    }

    #[test]
    fn expanded_by_default_shows_all_directories() {
        let mut progress = progress();
        progress.files.push(file("other/x.rs", "x"));
        let tree = tree_view(&progress, false);
        let names: Vec<_> = tree.rows.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"src"));
        assert!(names.contains(&"a.rs"));
        assert!(names.contains(&"b.rs"));
        assert!(names.contains(&"other"));
        assert!(names.contains(&"x.rs"));
        assert!(tree.opened.contains("other"));
        assert!(tree.opened.contains("src"));
        assert!(tree.fully_expanded);
        assert_eq!(tree.selected_file_path().as_deref(), Some("src/a.rs"));
    }

    #[test]
    fn toggle_expand_all_collapses_except_selection_ancestors() {
        let mut progress = progress();
        progress.files.push(file("other/x.rs", "x"));
        let mut tree = tree_view(&progress, false);
        assert!(tree.fully_expanded);
        assert!(tree.jump_to_path("src/b.rs"));
        tree.toggle_expand_all(&progress);
        let names: Vec<_> = tree.rows.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"b.rs"));
        assert!(!names.contains(&"x.rs"));
        assert!(tree.opened.contains("src"));
        assert!(!tree.opened.contains("other"));
        assert!(!tree.fully_expanded);
        assert_eq!(tree.selected_file_path().as_deref(), Some("src/b.rs"));
    }

    #[test]
    fn toggle_expand_all_restores_every_directory() {
        let mut progress = progress();
        progress.files.push(file("other/x.rs", "x"));
        let mut tree = tree_view(&progress, false);
        tree.toggle_expand_all(&progress);
        tree.toggle_expand_all(&progress);
        let names: Vec<_> = tree.rows.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"x.rs"));
        assert!(tree.opened.contains("src"));
        assert!(tree.opened.contains("other"));
        assert!(tree.fully_expanded);
        assert_eq!(tree.selected_file_path().as_deref(), Some("src/a.rs"));
    }

    #[test]
    fn footer_lists_fold_and_expand_shortcuts() {
        let mut progress = progress();
        progress.files.push(file("other/x.rs", "x"));
        let mut tree = tree_view(&progress, false);
        let keys = |tree: &TreeView| {
            tree_footer_hints(tree, en())
                .into_iter()
                .map(|(k, _)| k)
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&tree), ["Enter", "j/k", "h/l", "o", "Tab", "?"]);
        let hints = tree_footer_hints(&tree, en());
        assert_eq!(hints[3], ("o", "collapse"));

        tree.toggle_expand_all(&progress);
        let hints = tree_footer_hints(&tree, en());
        assert_eq!(hints[3], ("o", "expand"));

        tree.filter_editing = true;
        assert_eq!(keys(&tree), ["Esc", "n/N", "j/k", "Enter", "?"]);
    }

    #[test]
    fn hidden_paths_hide_files_and_rolls_up_empty_dirs() {
        let progress = progress();
        let mut hidden = std::collections::HashSet::new();
        hidden.insert("src/a.rs".to_string());
        hidden.insert("src/b.rs".to_string());
        let tree = TreeView::from_progress_full("demo", &progress, false, None, hidden);
        let names: Vec<_> = tree.rows.iter().map(|r| r.name.as_str()).collect();
        assert!(!names.contains(&"src"));
        assert!(!names.contains(&"a.rs"));
        assert!(!names.contains(&"b.rs"));
        assert!(names.contains(&"lib.rs"));
    }

    #[test]
    fn hidden_paths_still_hide_after_refresh() {
        let progress = progress();
        let mut hidden = std::collections::HashSet::new();
        hidden.insert("lib.rs".to_string());
        let mut tree = TreeView::from_progress_full("demo", &progress, false, None, hidden);
        tree.refresh_rows(&progress);
        let names: Vec<_> = tree.rows.iter().map(|r| r.name.as_str()).collect();
        assert!(!names.contains(&"lib.rs"));
        assert!(names.contains(&"a.rs"));
    }

    #[test]
    fn single_child_dir_chain_is_compressed() {
        let progress = RepoProgress {
            files: vec![file("a/b/c.rs", "c")],
        };
        let tree = tree_view(&progress, false);
        let dirs: Vec<_> = tree
            .rows
            .iter()
            .filter_map(|r| match &r.kind {
                TreeRowKind::Dir { path } => Some((r.name.as_str(), path.as_str(), r.depth)),
                TreeRowKind::File { .. } => None,
            })
            .collect();
        assert_eq!(dirs, vec![("a/b", "a/b", 0)]);
        assert_eq!(tree.rows.len(), 2);
        assert_eq!(tree.rows[1].depth, 1);
        assert_eq!(tree.selected_file_path().as_deref(), Some("a/b/c.rs"));
    }

    #[test]
    fn collapse_or_parent_walks_up_to_existing_row() {
        let progress = RepoProgress {
            files: vec![file("src/a/b/c.rs", "c"), file("src/other.rs", "o")],
        };
        let mut tree = tree_view(&progress, false);
        let idx = tree
            .rows
            .iter()
            .position(|r| matches!(&r.kind, TreeRowKind::Dir { path } if path == "src/a/b"))
            .expect("compressed src/a/b row");
        tree.selected = idx;
        tree.collapse_or_parent(&progress);
        assert!(!tree.opened.contains("src/a/b"));
        assert_eq!(tree.selected_dir_path().as_deref(), Some("src/a/b"));
        tree.collapse_or_parent(&progress);
        assert_eq!(tree.selected_dir_path().as_deref(), Some("src"));
    }

    #[test]
    fn elide_dir_name_keeps_first_and_last_segment() {
        assert_eq!(
            elide_dir_name("swift-scanner/pkg/SpecLinkSwiftScanner", 36),
            "swift-scanner/…/SpecLinkSwiftScanner"
        );
        assert_eq!(elide_dir_name("src/cli", 20), "src/cli");
    }

    #[test]
    fn list_height_subtracts_chrome_and_borders() {
        assert_eq!(list_height(Rect::new(0, 0, 80, 24)), 18);
        assert_eq!(list_height(Rect::new(0, 0, 80, 6)), 0);
        assert_eq!(list_height(Rect::new(0, 0, 80, 10)), 4);
    }

    #[test]
    fn header_label_counts_files_not_lines() {
        let progress = RepoProgress {
            files: vec![file("src/a.rs", "a\nb\nc"), file("lib.rs", "l")],
        };
        let tree = tree_view(&progress, false);
        let label = header_label(&tree, en());
        assert!(
            label.contains("0/2 files"),
            "expected file counts in {label}"
        );
        assert!(
            !label.contains("lines"),
            "label should not count lines: {label}"
        );
        assert_eq!(tree.overall, (0, 4));
        assert_eq!(tree.recommend.as_deref(), Some("src/a.rs"));
        assert_eq!(tree.recommend_lines, Some(3));
    }

    #[test]
    fn detail_row_summarizes_selected_file_and_dir() {
        let progress = progress();
        let mut tree = tree_view(&progress, false);
        assert_eq!(selected_detail(&tree, en()), "src/a.rs · 1 lines · 0 done");
        tree.move_by(-1);
        assert_eq!(selected_detail(&tree, en()), "src/ · 2 files · 0 done");
    }

    #[test]
    fn skipped_row_uses_short_label() {
        let mut progress = progress();
        progress
            .files
            .push(skipped_file("tests/foo.rs", SkipReason::TestFile));
        let mut tree = tree_view(&progress, false);
        tree.opened.insert("tests".into());
        tree.refresh_rows(&progress);
        let skipped = tree
            .rows
            .iter()
            .find(|r| r.name == "foo.rs")
            .expect("skipped row");
        assert!(!skipped.manual_skip);
        assert_eq!(skipped.skip_reason.as_ref(), Some(&SkipReason::TestFile));
    }
}
