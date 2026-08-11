//! Typing screen rendering.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::domain::typing::TypingSnapshot;

pub fn draw_typing(
    frame: &mut Frame,
    area: Rect,
    path: &str,
    file_label: &str,
    snap: &TypingSnapshot,
    now_ms: u64,
) {
    let block = Block::default()
        .title(format!(" {path}  {file_label} "))
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let body = render_body(snap, now_ms, panes[0].height as usize);
    frame.render_widget(Paragraph::new(body), panes[0]);

    let chars: Vec<char> = snap.target.chars().collect();
    let lines = split_lines(&chars);
    let total_lines = lines.len().max(1);
    let current_line = line_index_at(&lines, snap.cursor) + 1;
    let status = format!(
        "line {current_line}/{total_lines}   misses {}   Esc interrupt",
        snap.misses
    );
    frame.render_widget(Paragraph::new(status), panes[1]);
}

fn render_body(snap: &TypingSnapshot, now_ms: u64, height: usize) -> Vec<Line<'static>> {
    let chars: Vec<char> = snap.target.chars().collect();
    let lines = split_lines(&chars);
    let cursor_line = line_index_at(&lines, snap.cursor);
    let flash = snap
        .miss_until_ms
        .map(|until| now_ms < until)
        .unwrap_or(false);

    let height = height.max(1);
    let start = viewport_start(cursor_line, lines.len(), height);
    let end = (start + height).min(lines.len());

    let mut out = Vec::new();
    for (_li, line) in lines.iter().enumerate().take(end).skip(start) {
        let mut spans = Vec::new();
        for &(idx, ch) in line {
            let display = if ch == '\n' { '↵' } else { ch };
            let mut style = if idx < snap.cursor {
                Style::default().fg(Color::Green)
            } else if idx == snap.cursor {
                Style::default()
                    .fg(if flash { Color::Red } else { Color::Yellow })
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            if snap.auto_inserted.contains(&idx) && idx < snap.cursor {
                style = Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::DIM);
            }
            spans.push(Span::styled(display.to_string(), style));
        }
        if line.is_empty() {
            spans.push(Span::raw(""));
        }
        out.push(Line::from(spans));
    }
    out
}

/// Choose the first visible line so the whole file fits when possible,
/// otherwise keep the cursor visible with more upcoming context than past lines.
fn viewport_start(cursor_line: usize, total_lines: usize, height: usize) -> usize {
    if total_lines <= height {
        return 0;
    }
    // Near the top: pin to start so the file opens showing as much as possible.
    if cursor_line < height.saturating_sub(height / 3) {
        return 0;
    }
    // Near the bottom: pin to end.
    if cursor_line + height / 3 >= total_lines {
        return total_lines - height;
    }
    // Otherwise keep the cursor around the upper third so upcoming lines stay visible.
    cursor_line.saturating_sub(height / 3)
}

fn split_lines(chars: &[char]) -> Vec<Vec<(usize, char)>> {
    let mut lines = Vec::new();
    let mut cur = Vec::new();
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '\n' {
            cur.push((i, ch));
            lines.push(std::mem::take(&mut cur));
        } else {
            cur.push((i, ch));
        }
    }
    lines.push(cur);
    lines
}

fn line_index_at(lines: &[Vec<(usize, char)>], cursor: usize) -> usize {
    for (li, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let first = line[0].0;
        let last = line[line.len() - 1].0;
        if cursor >= first && cursor <= last + 1 {
            return li;
        }
    }
    lines.len().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_file_shows_from_top() {
        assert_eq!(viewport_start(0, 10, 20), 0);
        assert_eq!(viewport_start(5, 10, 20), 0);
    }

    #[test]
    fn start_of_long_file_pins_to_top() {
        assert_eq!(viewport_start(0, 100, 20), 0);
        assert_eq!(viewport_start(5, 100, 20), 0);
    }

    #[test]
    fn middle_keeps_upcoming_context() {
        // height/3 == 6 → start = 30 - 6 = 24
        assert_eq!(viewport_start(30, 100, 20), 24);
    }

    #[test]
    fn end_pins_to_bottom() {
        assert_eq!(viewport_start(95, 100, 20), 80);
    }
}
