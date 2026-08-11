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
    chunk_label: &str,
    snap: &TypingSnapshot,
    now_ms: u64,
) {
    let block = Block::default()
        .title(format!(" {path}  {chunk_label} "))
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let body = render_body(snap, now_ms, chunks[0].height as usize);
    frame.render_widget(Paragraph::new(body), chunks[0]);

    let progress = if snap.target.is_empty() {
        100
    } else {
        (snap.cursor * 100) / snap.target.chars().count().max(1)
    };
    let status = format!(
        "progress {progress}%   misses {}   Esc interrupt",
        snap.misses
    );
    frame.render_widget(Paragraph::new(status), chunks[1]);
}

fn render_body(snap: &TypingSnapshot, now_ms: u64, height: usize) -> Vec<Line<'static>> {
    let chars: Vec<char> = snap.target.chars().collect();
    let lines = split_lines(&chars);
    let cursor_line = line_index_at(&lines, snap.cursor);
    let flash = snap
        .miss_until_ms
        .map(|until| now_ms < until)
        .unwrap_or(false);

    // Keep current line roughly centered.
    let half = height / 2;
    let start = cursor_line.saturating_sub(half);
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
