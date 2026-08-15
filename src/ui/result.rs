//! Result screen.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::domain::content::TypingMetrics;
use crate::domain::dependency::FlowVia;
use crate::ui::i18n::UiStrings;
use crate::ui::theme;

#[derive(Debug, Clone)]
pub struct NextStep {
    pub index: usize,
    pub total: usize,
    pub path: String,
    pub via: Option<FlowVia>,
}

#[derive(Debug, Clone)]
pub struct ResultView {
    pub repo: String,
    pub path: String,
    pub completed: bool,
    pub metrics: TypingMetrics,
    pub file_done: bool,
    pub next: Option<NextStep>,
}

pub fn draw_result(frame: &mut Frame, area: Rect, view: &ResultView, t: &UiStrings) {
    theme::fill_background(frame, area);

    let card_width = (view.path.chars().count() as u16 + 8).clamp(46, area.width);
    let extra = if view.next.is_some() { 5 } else { 0 };
    let card = theme::centered_rect(area, card_width, 15 + extra);
    let title = if view.repo.is_empty() {
        view.path.clone()
    } else {
        format!("{} › {}", view.repo, view.path)
    };
    let block = theme::bordered_block(theme::title_line(&title));
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let (headline, color) = if view.file_done {
        (t.file_complete, theme::GREEN)
    } else {
        (t.session_complete, theme::GREEN)
    };

    let m = &view.metrics;
    // Fixed-width labels keep the value column aligned inside the card.
    let label_indent = inner.width.saturating_sub(30) / 2;
    let metric = move |label: &str, value: String| -> Line<'static> {
        Line::from(vec![
            Span::raw(" ".repeat(label_indent as usize)),
            Span::styled(format!("{label:>12}  "), Style::default().fg(theme::MUTED)),
            Span::styled(
                value,
                Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
            ),
        ])
    };

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            headline,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        Line::from(Span::styled(
            view.path.clone(),
            Style::default().fg(theme::MUTED),
        ))
        .alignment(Alignment::Center),
        Line::from(""),
        metric(t.accuracy, format!("{:.1}%", m.accuracy)),
        metric("KPM", format!("{:.0}", m.kpm)),
        metric("WPM", format!("{:.0}", m.wpm)),
        metric(t.keystrokes, format!("{}", m.keystrokes)),
        metric(t.misses, format!("{}", m.misses)),
        metric(t.time, format!("{:.1}s", m.elapsed_ms as f64 / 1000.0)),
        Line::from(""),
    ];
    if let Some(next) = &view.next {
        lines.push(
            Line::from(Span::styled(
                format!(
                    "── {}  {}/{} ──────────────",
                    t.next_section, next.index, next.total
                ),
                Style::default().fg(theme::MUTED),
            ))
            .alignment(Alignment::Center),
        );
        lines.push(
            Line::from(Span::styled(
                next.path.clone(),
                Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Center),
        );
        if let Some(via) = &next.via {
            lines.push(
                Line::from(Span::styled(
                    format!("← {}:{}  {}", via.importer, via.line, via.raw),
                    Style::default().fg(theme::MUTED),
                ))
                .alignment(Alignment::Center),
            );
        }
        lines.push(Line::from(""));
    }
    lines.push(
        theme::key_hints(&[
            ("Enter", t.next),
            ("r", t.retry),
            ("Esc", t.back),
            ("?", t.help),
        ])
        .alignment(Alignment::Center),
    );
    frame.render_widget(Paragraph::new(lines), inner);
}
