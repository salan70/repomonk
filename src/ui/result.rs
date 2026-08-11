//! Result screen.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::domain::content::TypingMetrics;

#[derive(Debug, Clone)]
pub struct ResultView {
    pub path: String,
    pub completed: bool,
    pub metrics: TypingMetrics,
    pub file_done: bool,
}

pub fn draw_result(frame: &mut Frame, area: Rect, view: &ResultView) {
    let block = Block::default().title(" Result ").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(6),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .split(inner);

    let headline = if view.completed {
        if view.file_done {
            "File complete"
        } else {
            "Session complete"
        }
    } else {
        "Interrupted"
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            headline,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ))),
        layout[0],
    );

    let m = &view.metrics;
    let body = vec![
        Line::from(format!("file: {}", view.path)),
        Line::from(format!(
            "accuracy: {:.1}%   KPM: {:.0}   WPM: {:.0}",
            m.accuracy, m.kpm, m.wpm
        )),
        Line::from(format!(
            "keystrokes: {}   misses: {}   time: {:.1}s",
            m.keystrokes,
            m.misses,
            m.elapsed_ms as f64 / 1000.0
        )),
    ];
    frame.render_widget(Paragraph::new(body), layout[1]);

    frame.render_widget(Paragraph::new("Enter/Esc: back to tree"), layout[2]);
}
