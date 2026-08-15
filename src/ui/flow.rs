//! Flow overlay (Tree `e`): choose progress mode and entry point.
//!
//! Rendered as a floating dialog over the Tree, the same way File types floats.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::config::ProgressMode;
use crate::domain::entry::EntryCandidate;
use crate::ui::i18n::UiStrings;
use crate::ui::theme;

#[derive(Debug, Clone, PartialEq, Eq)]
enum FlowRow {
    ModeFlow,
    ModeManual,
    Entry { path: String, reason: &'static str },
}

#[derive(Debug, Clone)]
pub struct FlowView {
    pub repo_label: String,
    rows: Vec<FlowRow>,
    pub selected: usize,
    pub mode: ProgressMode,
    pub entry: Option<String>,
    pub flow_enabled: bool,
    pub disabled_reason: Option<String>,
    pub file_count: usize,
}

impl FlowView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo_label: &str,
        mode: ProgressMode,
        entry: Option<String>,
        candidates: Vec<EntryCandidate>,
        tree_selection: Option<String>,
        flow_enabled: bool,
        disabled_reason: Option<String>,
        file_count: usize,
    ) -> Self {
        let mut rows = vec![FlowRow::ModeFlow, FlowRow::ModeManual];
        let mut seen: std::collections::HashSet<String> =
            candidates.iter().map(|c| c.path.clone()).collect();
        for candidate in candidates {
            rows.push(FlowRow::Entry {
                path: candidate.path,
                reason: candidate.reason,
            });
        }
        if let Some(path) = tree_selection {
            if seen.insert(path.clone()) {
                rows.push(FlowRow::Entry {
                    path,
                    reason: "selected in tree",
                });
            }
        }

        let mode = if flow_enabled {
            mode
        } else {
            ProgressMode::Manual
        };
        let mut view = Self {
            repo_label: repo_label.to_string(),
            rows,
            selected: 0,
            mode,
            entry,
            flow_enabled,
            disabled_reason,
            file_count,
        };
        view.selected = if view.mode == ProgressMode::Flow && view.flow_enabled {
            0
        } else {
            1
        };
        view
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() || delta == 0 {
            return;
        }
        let len = self.rows.len() as isize;
        let direction = delta.signum();
        let mut next = (self.selected as isize + delta).rem_euclid(len);
        for _ in 0..self.rows.len() {
            if self.row_enabled(next as usize) {
                self.selected = next as usize;
                return;
            }
            next = (next + direction).rem_euclid(len);
        }
    }

    pub fn activate_selected(&mut self) {
        match self.rows.get(self.selected) {
            Some(FlowRow::ModeFlow) if self.flow_enabled => {
                self.mode = ProgressMode::Flow;
            }
            Some(FlowRow::ModeManual) => {
                self.mode = ProgressMode::Manual;
            }
            Some(FlowRow::Entry { path, .. }) => {
                self.entry = Some(path.clone());
            }
            _ => {}
        }
    }

    fn row_enabled(&self, index: usize) -> bool {
        match self.rows.get(index) {
            Some(FlowRow::ModeFlow) => self.flow_enabled,
            Some(_) => true,
            None => false,
        }
    }
}

fn dialog_rect(area: Rect, row_count: usize) -> Rect {
    let width = area.width.saturating_sub(8).min(72);
    let height = (row_count as u16 + 8).clamp(10, area.height.saturating_sub(2));
    theme::centered_rect(area, width, height)
}

pub fn draw_flow(frame: &mut Frame, area: Rect, view: &FlowView, t: &UiStrings) {
    let card = dialog_rect(area, view.rows.len());
    frame.render_widget(Clear, card);
    let title = format!("{} — {}", t.title_flow, view.repo_label);
    let block = theme::bordered_block(theme::title_line(&title));
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

    let entries_dim = view.mode != ProgressMode::Flow;
    let mut items = Vec::new();
    let mut last_kind: Option<&str> = None;
    for (index, row) in view.rows.iter().enumerate() {
        let kind = match row {
            FlowRow::ModeFlow | FlowRow::ModeManual => "mode",
            FlowRow::Entry { .. } => "entry",
        };
        if last_kind != Some(kind) {
            if last_kind.is_some() {
                items.push(ListItem::new(Line::from("")));
            }
            let heading = match kind {
                "mode" => t.flow_mode,
                _ => t.flow_entry,
            };
            items.push(ListItem::new(Line::from(Span::styled(
                format!("  {heading}"),
                Style::default()
                    .fg(theme::BLUE)
                    .add_modifier(Modifier::BOLD),
            ))));
            last_kind = Some(kind);
        }

        let selected_here = index == view.selected;
        let marker = if selected_here { "▸ " } else { "  " };
        let (bullet, label, extra, enabled) = match row {
            FlowRow::ModeFlow => {
                let bullet = if view.mode == ProgressMode::Flow {
                    "●"
                } else {
                    "○"
                };
                let extra = format!("{:>5} {}", view.file_count, t.files);
                (bullet, t.flow_follow.to_string(), extra, view.flow_enabled)
            }
            FlowRow::ModeManual => {
                let bullet = if view.mode == ProgressMode::Manual {
                    "●"
                } else {
                    "○"
                };
                (bullet, t.flow_manual.to_string(), String::new(), true)
            }
            FlowRow::Entry { path, reason } => {
                let bullet = if view.entry.as_deref() == Some(path.as_str()) {
                    "●"
                } else {
                    "○"
                };
                (
                    bullet,
                    format!("{path:<20}  {}", t.entry_reason(reason)),
                    String::new(),
                    true,
                )
            }
        };
        let color = if !enabled || (matches!(row, FlowRow::Entry { .. }) && entries_dim) {
            theme::MUTED
        } else {
            theme::FG
        };
        let mut spans = vec![
            Span::styled(marker.to_string(), Style::default().fg(theme::CYAN)),
            Span::styled(format!(" {bullet} {label}"), Style::default().fg(color)),
        ];
        if !extra.is_empty() {
            spans.push(Span::styled(
                format!("  {extra}"),
                Style::default().fg(theme::MUTED),
            ));
        }
        items.push(ListItem::new(Line::from(spans)));
    }
    if let Some(reason) = &view.disabled_reason {
        items.push(ListItem::new(Line::from("")));
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  {reason}"),
            Style::default().fg(theme::MUTED),
        ))));
    }

    // ListState indexes into `items`, which includes section headers and blanks.
    // Map the data-row selection onto the rendered item index.
    let highlight = rendered_index(view);
    let list = List::new(items).highlight_style(
        Style::default()
            .bg(theme::CURRENT_LINE_BG)
            .fg(theme::FG)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default();
    state.select(Some(highlight));
    frame.render_stateful_widget(list, panes[0], &mut state);

    frame.render_widget(
        Paragraph::new(theme::key_hints(&[
            ("j/k", t.move_),
            ("Enter/Space", t.select),
            ("Esc/q", t.apply_close),
        ])),
        panes[1],
    );
}

fn rendered_index(view: &FlowView) -> usize {
    let mut rendered = 0usize;
    let mut last_kind: Option<&str> = None;
    for (index, row) in view.rows.iter().enumerate() {
        let kind = match row {
            FlowRow::ModeFlow | FlowRow::ModeManual => "mode",
            FlowRow::Entry { .. } => "entry",
        };
        if last_kind != Some(kind) {
            if last_kind.is_some() {
                rendered += 1; // blank
            }
            rendered += 1; // heading
            last_kind = Some(kind);
        }
        if index == view.selected {
            return rendered;
        }
        rendered += 1;
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(flow_enabled: bool) -> FlowView {
        FlowView::new(
            "demo",
            ProgressMode::Flow,
            Some("src/main.rs".into()),
            vec![
                EntryCandidate {
                    path: "src/main.rs".into(),
                    reason: "bin (Cargo.toml)",
                },
                EntryCandidate {
                    path: "src/lib.rs".into(),
                    reason: "crate root",
                },
            ],
            Some("src/app/mod.rs".into()),
            flow_enabled,
            if flow_enabled {
                None
            } else {
                Some("no import-analyzable language found".into())
            },
            42,
        )
    }

    #[test]
    fn appends_tree_selection_and_wraps_across_sections() {
        let mut view = view(true);
        assert_eq!(view.rows.len(), 5);
        assert_eq!(view.selected, 0);
        view.move_by(-1);
        assert_eq!(view.selected, 4);
        view.activate_selected();
        assert_eq!(view.entry.as_deref(), Some("src/app/mod.rs"));
        view.move_by(1);
        view.activate_selected();
        assert_eq!(view.mode, ProgressMode::Flow);
    }

    #[test]
    fn disabled_flow_skips_flow_row_and_selects_manual() {
        let mut view = view(false);
        assert_eq!(view.mode, ProgressMode::Manual);
        assert_eq!(view.selected, 1);
        view.move_by(-1);
        assert_ne!(view.selected, 0);
        view.selected = 0;
        view.activate_selected();
        assert_eq!(view.mode, ProgressMode::Manual);
    }
}
