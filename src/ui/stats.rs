//! Achievement-only stats screen (home `g`).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::store::{GlobalSummary, RecentRepo};
use crate::ui::theme;

#[derive(Debug, Clone)]
pub struct StatsView {
    pub summary: GlobalSummary,
    pub repos: Vec<RecentRepo>,
}

impl StatsView {
    pub fn new(summary: GlobalSummary, repos: Vec<RecentRepo>) -> Self {
        Self { summary, repos }
    }
}

pub fn draw_stats(frame: &mut Frame, area: Rect, view: &StatsView) {
    theme::fill_background(frame, area);
    let block = theme::bordered_block(theme::title_line("Stats"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

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
    frame.render_widget(List::new(items).style(theme::base_style()), panes[1]);

    frame.render_widget(
        Paragraph::new(theme::key_hints(&[("Esc", "back"), ("q", "quit")])),
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
