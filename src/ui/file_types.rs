//! File types overlay (Tree `t`): per-extension / per-filename include toggles.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::domain::content::{FileStatus, RepoProgress};
use crate::domain::file_type::{file_type_key, file_type_stats, FileTypePrefs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTypeEntry {
    pub key: String,
    pub files: usize,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct FileTypesView {
    pub repo_label: String,
    pub entries: Vec<FileTypeEntry>,
    pub selected: usize,
}

impl FileTypesView {
    pub fn from_progress(repo_label: &str, progress: &RepoProgress, saved: &FileTypePrefs) -> Self {
        let stats = file_type_stats(progress);
        let entries = stats
            .into_iter()
            .map(|stat| {
                let default_enabled = progress.files.iter().any(|f| {
                    file_type_key(&f.relative_path) == stat.key
                        && f.derive_status() != FileStatus::Skipped
                });
                let enabled = saved.get(&stat.key).unwrap_or(default_enabled);
                FileTypeEntry {
                    key: stat.key,
                    files: stat.files,
                    enabled,
                }
            })
            .collect();
        Self {
            repo_label: repo_label.to_string(),
            entries,
            selected: 0,
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let len = self.entries.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }

    pub fn toggle_selected(&mut self) {
        if let Some(entry) = self.entries.get_mut(self.selected) {
            entry.enabled = !entry.enabled;
        }
    }

    pub fn to_prefs(&self) -> Vec<(String, bool)> {
        self.entries
            .iter()
            .map(|e| (e.key.clone(), e.enabled))
            .collect()
    }
}

pub fn draw_file_types(frame: &mut Frame, area: Rect, view: &FileTypesView) {
    crate::ui::theme::fill_background(frame, area);
    let title = format!("File types — {}", view.repo_label);
    let block = crate::ui::theme::bordered_block(crate::ui::theme::title_line(&title));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

    let items: Vec<ListItem> = view
        .entries
        .iter()
        .map(|entry| {
            let checkbox = if entry.enabled { "[x]" } else { "[ ]" };
            let plural = if entry.files == 1 { "file" } else { "files" };
            let label = format!("  {checkbox} {:<14} {:>5} {plural}", entry.key, entry.files);
            ListItem::new(Line::from(Span::styled(
                label,
                Style::default().fg(if entry.enabled {
                    crate::ui::theme::FG
                } else {
                    crate::ui::theme::MUTED
                }),
            )))
        })
        .collect();

    let list = List::new(items).highlight_style(
        Style::default()
            .bg(crate::ui::theme::CURRENT_LINE_BG)
            .fg(crate::ui::theme::FG)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default();
    if !view.entries.is_empty() {
        state.select(Some(view.selected));
    }
    frame.render_stateful_widget(list, panes[0], &mut state);

    frame.render_widget(
        Paragraph::new(crate::ui::theme::key_hints(&[
            ("Space/Enter", "toggle"),
            ("j/k", "move"),
            ("Esc/q", "apply & close (rescans)"),
        ])),
        panes[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::content::{Chunk, ChunkCompletion, ChunkProgress, FileProgress};

    fn typeable(path: &str) -> FileProgress {
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
                    normalized: "x".into(),
                    hash: "h".into(),
                },
                completion: ChunkCompletion::Incomplete,
                checkpoint: None,
                id: Some(1),
            }],
        }
    }

    fn skipped(path: &str, reason: crate::domain::content::SkipReason) -> FileProgress {
        FileProgress {
            relative_path: path.into(),
            status: FileStatus::Skipped,
            skip_reason: Some(reason),
            manual_override: None,
            chunks: Vec::new(),
        }
    }

    #[test]
    fn default_state_reflects_current_auto_detection() {
        let progress = RepoProgress {
            files: vec![
                typeable("src/main.rs"),
                skipped("README.md", crate::domain::content::SkipReason::ConfigFile),
                typeable("LICENSE"),
            ],
        };
        let view = FileTypesView::from_progress("repo", &progress, &FileTypePrefs::default());
        let rs = view.entries.iter().find(|e| e.key == ".rs").unwrap();
        let md = view.entries.iter().find(|e| e.key == ".md").unwrap();
        let license = view.entries.iter().find(|e| e.key == "LICENSE").unwrap();
        assert!(rs.enabled);
        assert!(!md.enabled);
        assert!(license.enabled);
    }

    #[test]
    fn saved_prefs_override_default() {
        let progress = RepoProgress {
            files: vec![skipped(
                "README.md",
                crate::domain::content::SkipReason::ConfigFile,
            )],
        };
        let mut map = std::collections::HashMap::new();
        map.insert(".md".to_string(), true);
        let view = FileTypesView::from_progress("repo", &progress, &FileTypePrefs::new(map));
        assert!(view.entries[0].enabled);
    }

    #[test]
    fn toggle_and_move_wrap() {
        let progress = RepoProgress {
            files: vec![typeable("a.rs"), typeable("b.ts")],
        };
        let mut view = FileTypesView::from_progress("repo", &progress, &FileTypePrefs::default());
        assert_eq!(view.entries.len(), 2);
        view.toggle_selected();
        assert!(!view.entries[0].enabled);
        view.move_by(-1);
        assert_eq!(view.selected, 1);
        view.move_by(1);
        assert_eq!(view.selected, 0);
    }
}
