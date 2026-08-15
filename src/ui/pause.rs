//! Pause overlay shown over the Typing place.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::ui::i18n::UiStrings;
use crate::ui::theme;

pub fn draw_pause(frame: &mut Frame, area: Rect, t: &UiStrings) {
    let card = theme::centered_rect(area, 56, 9);
    frame.render_widget(Clear, card);
    let block = theme::bordered_block(theme::title_line(t.title_paused));
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            t.pause_timer,
            Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        Line::from(Span::styled(
            t.pause_idle,
            Style::default().fg(theme::MUTED),
        ))
        .alignment(Alignment::Center),
        Line::from(""),
        theme::key_hints(&[
            ("Esc/Enter", t.resume),
            ("r", t.retry),
            ("t", t.tree),
            ("?", t.help),
        ])
        .alignment(Alignment::Center),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}
