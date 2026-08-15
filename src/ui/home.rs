//! Home screen: logo, Recent list, Summary, and Search modal overlay.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::store::{GlobalSummary, RecentRepo};
use crate::ui::i18n::UiStrings;
use crate::ui::search::SearchState;
use crate::ui::splash::{self, LOGO};
use crate::ui::theme;

#[derive(Debug, Clone)]
pub struct HomeView {
    pub recent: Vec<RecentRepo>,
    pub summary: GlobalSummary,
    pub selected: usize,
    pub error: Option<String>,
}

impl HomeView {
    pub fn new(recent: Vec<RecentRepo>, summary: GlobalSummary) -> Self {
        Self {
            recent,
            summary,
            selected: 0,
            error: None,
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.recent.is_empty() {
            return;
        }
        let len = self.recent.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }

    pub fn selected_input(&self) -> Option<&str> {
        self.recent.get(self.selected).map(|r| r.input.as_str())
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.recent.len().saturating_sub(1);
    }
}

pub fn draw_home(
    frame: &mut Frame,
    area: Rect,
    view: &HomeView,
    logo_elapsed_ms: u64,
    t: &UiStrings,
) {
    theme::fill_background(frame, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(LOGO.len() as u16 + 2),
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .split(area);

    splash::draw_animated_logo(frame, chunks[0], logo_elapsed_ms);
    draw_summary(frame, chunks[1], &view.summary, t);
    draw_recent(frame, chunks[2], view, t);
    if let Some(err) = &view.error {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("  {err}"),
                Style::default().fg(theme::RED).bg(theme::BG),
            ))),
            chunks[3],
        );
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                t.home_hint,
                Style::default().fg(theme::MUTED).bg(theme::BG),
            ))),
            chunks[3],
        );
    }
    frame.render_widget(
        Paragraph::new(theme::key_hints(&[
            ("Enter", t.open),
            ("j/k", t.select),
            ("/", t.search),
            ("q", t.quit),
            ("?", t.help),
        ])),
        chunks[4],
    );
}

fn draw_summary(frame: &mut Frame, area: Rect, summary: &GlobalSummary, t: &UiStrings) {
    let text = format!(
        "  {}{sep}{} {}{sep}{} {}{sep}{} {}",
        t.summary,
        summary.completed_files,
        t.files_done,
        summary.completed_chunks,
        t.chunks_done,
        summary.streak_days,
        t.day_streak,
        sep = theme::FIELD_SEP,
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(theme::MUTED).bg(theme::BG),
        )))
        .alignment(Alignment::Left),
        area,
    );
}

fn draw_recent(frame: &mut Frame, area: Rect, view: &HomeView, t: &UiStrings) {
    let block = theme::bordered_block(theme::title_line(t.title_recent));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if view.recent.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                t.home_empty,
                Style::default().fg(theme::MUTED),
            )))
            .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    let items: Vec<ListItem> = view
        .recent
        .iter()
        .map(|r| ListItem::new(recent_line(r, t)))
        .collect();
    let mut state = ListState::default();
    state.select(Some(view.selected.min(view.recent.len() - 1)));
    let list = List::new(items)
        .highlight_style(Style::default().bg(theme::SELECTION_BG))
        .style(theme::base_style());
    frame.render_stateful_widget(list, inner, &mut state);
}

fn recent_line(r: &RecentRepo, t: &UiStrings) -> Line<'static> {
    let ratio = if r.total_lines == 0 {
        0.0
    } else {
        r.done_lines as f64 / r.total_lines as f64
    };
    let bar = progress_bar(ratio, 10);
    let pct = format!("{:>3.0}%", ratio * 100.0);
    Line::from(vec![
        Span::styled(
            format!(" {}", r.display_name),
            Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  {bar} {pct}  {}/{} {}",
                r.done_lines, r.total_lines, t.lines
            ),
            Style::default().fg(theme::MUTED),
        ),
    ])
}

fn progress_bar(ratio: f64, width: usize) -> String {
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

/// Draw the Search modal centered over the home screen.
pub fn draw_search_modal(frame: &mut Frame, area: Rect, search: &SearchState, t: &UiStrings) {
    let modal = theme::centered_rect(area, area.width.saturating_sub(8).min(72), 16);
    frame.render_widget(Clear, modal);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::CYAN).bg(theme::BG))
        .style(theme::base_style())
        .title(theme::title_line(t.title_search));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(theme::CYAN)),
            Span::styled(
                search.query.clone(),
                Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
            ),
            Span::styled("▌", Style::default().fg(theme::CYAN)),
        ])),
        panes[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            t.search_hint,
            Style::default().fg(theme::MUTED),
        ))),
        panes[1],
    );

    let items: Vec<ListItem> = search
        .hits
        .iter()
        .map(|h| {
            ListItem::new(Line::from(Span::styled(
                format!(
                    " {}{}",
                    h.label,
                    if h.open_hint {
                        t.search_open_suffix
                    } else {
                        ""
                    }
                ),
                Style::default().fg(theme::FG),
            )))
        })
        .collect();
    let mut state = ListState::default();
    if !search.hits.is_empty() {
        state.select(Some(search.selected));
    }
    frame.render_stateful_widget(
        List::new(items)
            .highlight_style(Style::default().bg(theme::SELECTION_BG))
            .style(theme::base_style()),
        panes[2],
        &mut state,
    );

    let confirm_hint = if search.can_confirm() {
        ("Enter", t.open)
    } else {
        ("Enter", t.need_a_match)
    };
    frame.render_widget(
        Paragraph::new(theme::key_hints(&[
            confirm_hint,
            ("↑/↓", t.select),
            ("Esc", t.close),
            ("?", t.help),
        ])),
        panes[3],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_bar_bounds() {
        assert_eq!(progress_bar(0.0, 4), "░░░░");
        assert_eq!(progress_bar(1.0, 4), "████");
        assert_eq!(progress_bar(0.5, 4), "██░░");
    }

    #[test]
    fn home_move_wraps() {
        let mut view = HomeView::new(
            vec![
                RecentRepo {
                    id: 1,
                    identity: "a".into(),
                    display_name: "a".into(),
                    input: "a".into(),
                    root_path: String::new(),
                    last_opened_at: String::new(),
                    done_lines: 0,
                    total_lines: 1,
                },
                RecentRepo {
                    id: 2,
                    identity: "b".into(),
                    display_name: "b".into(),
                    input: "b".into(),
                    root_path: String::new(),
                    last_opened_at: String::new(),
                    done_lines: 0,
                    total_lines: 1,
                },
            ],
            GlobalSummary::default(),
        );
        view.move_by(1);
        assert_eq!(view.selected, 1);
        view.move_by(1);
        assert_eq!(view.selected, 0);
    }
}
