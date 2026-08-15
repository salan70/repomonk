//! Achievement-only stats overlay (`S` / home `g`).
//!
//! Rendered as a floating dialog over the current place, the same way File types floats.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::store::{GlobalSummary, RecentRepo};
use crate::ui::theme;

#[derive(Debug, Clone)]
pub struct StatsView {
    pub summary: GlobalSummary,
    pub repos: Vec<RecentRepo>,
    pub selected: usize,
}

impl StatsView {
    pub fn move_by(&mut self, delta: isize) {
        if self.repos.is_empty() {
            return;
        }
        let len = self.repos.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.repos.len().saturating_sub(1);
    }

    pub fn new(summary: GlobalSummary, repos: Vec<RecentRepo>) -> Self {
        Self {
            summary,
            repos,
            selected: 0,
        }
    }
}

fn dialog_rect(area: Rect, repo_count: usize) -> Rect {
    let width = area.width.saturating_sub(8).min(72);
    let height = (repo_count.max(1) as u16 + 10).clamp(12, area.height.saturating_sub(2));
    theme::centered_rect(area, width, height)
}

pub fn draw_stats(frame: &mut Frame, area: Rect, view: &StatsView) {
    let card = dialog_rect(area, view.repos.len());
    frame.render_widget(Clear, card);
    let block = theme::bordered_block(theme::title_line("Stats"));
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

    let summary_lines = vec![
        Line::from(Span::styled(
            " Achievement",
            Style::default()
                .fg(theme::BLUE)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  files done   ", Style::default().fg(theme::MUTED)),
            Span::styled(
                format!("{}", view.summary.completed_files),
                Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  chunks done  ", Style::default().fg(theme::MUTED)),
            Span::styled(
                format!("{}", view.summary.completed_chunks),
                Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  streak       ", Style::default().fg(theme::MUTED)),
            Span::styled(
                format!("{} days", view.summary.streak_days),
                Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(summary_lines), panes[0]);

    let items: Vec<ListItem> = if view.repos.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  (no repositories)",
            Style::default().fg(theme::MUTED),
        )))]
    } else {
        view.repos
            .iter()
            .map(|r| {
                let ratio = if r.total_lines == 0 {
                    0.0
                } else {
                    r.done_lines as f64 / r.total_lines as f64
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<28}", truncate(&r.display_name, 28)),
                        Style::default().fg(theme::FG),
                    ),
                    Span::styled(
                        format!(
                            "{:>3.0}%  {}/{} lines",
                            ratio * 100.0,
                            r.done_lines,
                            r.total_lines
                        ),
                        Style::default().fg(theme::MUTED),
                    ),
                ]))
            })
            .collect()
    };
    let mut state = ListState::default();
    if !view.repos.is_empty() {
        state.select(Some(view.selected.min(view.repos.len() - 1)));
    }
    frame.render_stateful_widget(
        List::new(items)
            .highlight_style(Style::default().bg(theme::SELECTION_BG))
            .style(theme::base_style()),
        panes[1],
        &mut state,
    );

    frame.render_widget(
        Paragraph::new(theme::key_hints(&[
            ("j/k", "scroll"),
            ("Esc", "close"),
            ("?", "help"),
        ])),
        panes[2],
    );
}

fn truncate(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}
