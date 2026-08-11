//! Result screen.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::domain::content::TypingMetrics;
use crate::ui::theme;

#[derive(Debug, Clone)]
pub struct ResultView {
    pub path: String,
    pub completed: bool,
    pub metrics: TypingMetrics,
    pub file_done: bool,
}

pub fn draw_result(frame: &mut Frame, area: Rect, view: &ResultView) {
    theme::fill_background(frame, area);

    let card_width = (view.path.chars().count() as u16 + 8).clamp(46, area.width);
    let card = theme::centered_rect(area, card_width, 15);
    let block = theme::bordered_block(theme::title_line("Result"));
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let (headline, color) = if view.completed {
        if view.file_done {
            ("✓ File complete", theme::GREEN)
        } else {
            ("✓ Session complete", theme::GREEN)
        }
    } else {
        ("✗ Interrupted", theme::YELLOW)
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

    let lines = vec![
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
        metric("accuracy", format!("{:.1}%", m.accuracy)),
        metric("KPM", format!("{:.0}", m.kpm)),
        metric("WPM", format!("{:.0}", m.wpm)),
        metric("keystrokes", format!("{}", m.keystrokes)),
        metric("misses", format!("{}", m.misses)),
        metric("time", format!("{:.1}s", m.elapsed_ms as f64 / 1000.0)),
        Line::from(""),
        theme::key_hints(&[("Enter/Esc", "back to tree"), ("q", "quit")])
            .alignment(Alignment::Center),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}
