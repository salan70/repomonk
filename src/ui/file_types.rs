//! File types overlay (Tree `t`): per-extension / per-filename include/exclude/hide toggles.
//!
//! Rendered as a floating dialog over the Tree, the same way Pause floats over Typing.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::domain::content::{FileStatus, RepoProgress};
use crate::domain::file_type::{file_type_key, file_type_stats, FileTypePrefs, FileTypeState};
use crate::ui::i18n::UiStrings;
use crate::ui::theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTypeEntry {
    pub key: String,
    pub files: usize,
    pub state: FileTypeState,
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
                let default_state = if !stat.has_extension {
                    // Bare names (LICENSE, Makefile) and dotfiles (.gitignore) default to
                    // excluded: they are rarely worth typing and easy to opt back in.
                    FileTypeState::Excluded
                } else if progress.files.iter().any(|f| {
                    file_type_key(&f.relative_path) == stat.key
                        && f.derive_status() != FileStatus::Skipped
                }) {
                    FileTypeState::Included
                } else {
                    FileTypeState::Excluded
                };
                let state = saved.get(&stat.key).unwrap_or(default_state);
                FileTypeEntry {
                    key: stat.key,
                    files: stat.files,
                    state,
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

    /// Included -> Excluded -> Hidden -> Included.
    pub fn cycle_selected(&mut self) {
        if let Some(entry) = self.entries.get_mut(self.selected) {
            entry.state = entry.state.cycle();
        }
    }

    pub fn to_prefs(&self) -> Vec<(String, FileTypeState)> {
        self.entries
            .iter()
            .map(|e| (e.key.clone(), e.state))
            .collect()
    }
}

fn dialog_rect(area: Rect, entry_count: usize) -> Rect {
    let width = area.width.saturating_sub(8).min(60);
    let height = (entry_count as u16 + 6).clamp(8, area.height.saturating_sub(2));
    theme::centered_rect(area, width, height)
}

pub fn draw_file_types(frame: &mut Frame, area: Rect, view: &FileTypesView, t: &UiStrings) {
    let card = dialog_rect(area, view.entries.len());
    frame.render_widget(Clear, card);
    let title = format!("{} — {}", t.title_file_types, view.repo_label);
    let block = theme::bordered_block(theme::title_line(&title));
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

    let items: Vec<ListItem> = view
        .entries
        .iter()
        .map(|entry| {
            let (checkbox, color, suffix) = match entry.state {
                FileTypeState::Included => ("[x]", theme::FG, ""),
                FileTypeState::Excluded => ("[ ]", theme::MUTED, ""),
                FileTypeState::Hidden => ("[-]", theme::MUTED, t.file_types_hidden),
            };
            let plural = if entry.files == 1 {
                t.file_one
            } else {
                t.file_many
            };
            let label = format!(
                "  {checkbox} {:<14} {:>5} {plural}{suffix}",
                entry.key, entry.files
            );
            ListItem::new(Line::from(Span::styled(label, Style::default().fg(color))))
        })
        .collect();

    let list = List::new(items).highlight_style(
        Style::default()
            .bg(theme::CURRENT_LINE_BG)
            .fg(theme::FG)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default();
    if !view.entries.is_empty() {
        state.select(Some(view.selected));
    }
    frame.render_stateful_widget(list, panes[0], &mut state);

    frame.render_widget(
        Paragraph::new(theme::key_hints(&[
            ("Space/Enter", t.cycle),
            ("j/k", t.move_),
            ("Esc/q", t.apply_close),
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
        assert_eq!(rs.state, FileTypeState::Included);
        assert_eq!(md.state, FileTypeState::Excluded);
        // Extensionless types default to Excluded even though LICENSE currently
        // scans as typeable (no CONFIG_NAMES/CONFIG_EXTENSIONS entry catches it).
        assert_eq!(license.state, FileTypeState::Excluded);
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
        map.insert(".md".to_string(), FileTypeState::Hidden);
        let view = FileTypesView::from_progress("repo", &progress, &FileTypePrefs::new(map));
        assert_eq!(view.entries[0].state, FileTypeState::Hidden);
    }

    #[test]
    fn cycle_and_move_wrap() {
        let progress = RepoProgress {
            files: vec![typeable("a.rs"), typeable("b.ts")],
        };
        let mut view = FileTypesView::from_progress("repo", &progress, &FileTypePrefs::default());
        assert_eq!(view.entries.len(), 2);
        assert_eq!(view.entries[0].state, FileTypeState::Included);
        view.cycle_selected();
        assert_eq!(view.entries[0].state, FileTypeState::Excluded);
        view.cycle_selected();
        assert_eq!(view.entries[0].state, FileTypeState::Hidden);
        view.cycle_selected();
        assert_eq!(view.entries[0].state, FileTypeState::Included);
        view.move_by(-1);
        assert_eq!(view.selected, 1);
        view.move_by(1);
        assert_eq!(view.selected, 0);
    }
}
