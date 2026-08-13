//! Pause overlay shown over the Typing place.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::ui::theme;

pub fn draw_pause(frame: &mut Frame, area: Rect) {
    let card = theme::centered_rect(area, 46, 9);
    frame.render_widget(Clear, card);
    let block = theme::bordered_block(theme::title_line("Paused"));
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Timer keeps running",
            Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        Line::from(Span::styled(
            "Idle time still counts toward this session",
            Style::default().fg(theme::MUTED),
        ))
        .alignment(Alignment::Center),
        Line::from(""),
        theme::key_hints(&[
            ("Esc/Enter", "resume"),
            ("r", "retry"),
            ("t", "tree"),
            ("?", "help"),
        ])
        .alignment(Alignment::Center),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}
