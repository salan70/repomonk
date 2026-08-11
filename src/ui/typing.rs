//! Typing screen rendering.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph};
use ratatui::Frame;

use crate::config::FxPreset;
use crate::domain::typing::TypingSnapshot;
use crate::ui::fx::FxState;
use crate::ui::theme;

/// Cursor glow color per no-miss streak tier (0..=3).
fn tier_color(tier: u8) -> Color {
    match tier {
        0 => theme::YELLOW,
        1 => theme::CYAN,
        2 => theme::BLUE,
        _ => theme::MAGENTA,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_typing(
    frame: &mut Frame,
    area: Rect,
    path: &str,
    file_label: &str,
    snap: &TypingSnapshot,
    syntax_colors: Option<&[Color]>,
    now_ms: u64,
    fx: Option<&FxState>,
    show_live_speed: bool,
) {
    theme::fill_background(frame, area);

    let title = Line::from(vec![
        Span::styled(
            format!(" {path} "),
            Style::default()
                .fg(theme::BLUE)
                .bg(theme::BG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {file_label} "),
            Style::default().fg(theme::MUTED).bg(theme::BG),
        ),
    ]);
    let block = theme::bordered_block(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

    // Thin per-character progress gauge.
    let total_chars = snap.target.chars().count().max(1);
    let ratio = (snap.cursor as f64 / total_chars as f64).clamp(0.0, 1.0);
    let gauge = Gauge::default()
        .ratio(ratio)
        .use_unicode(true)
        .gauge_style(Style::default().fg(theme::BLUE).bg(theme::CURRENT_LINE_BG))
        .label(Span::styled(
            format!("{:.0}%", ratio * 100.0),
            Style::default().fg(theme::FG),
        ));
    let gauge_area = Rect {
        x: panes[0].x + 1,
        y: panes[0].y,
        width: panes[0].width.saturating_sub(2),
        height: panes[0].height,
    };
    frame.render_widget(gauge, gauge_area);

    let body = render_body(snap, syntax_colors, now_ms, panes[1].height as usize, fx);
    frame.render_widget(Paragraph::new(body), panes[1]);

    let chars: Vec<char> = snap.target.chars().collect();
    let lines = split_lines(&chars);
    let total_lines = lines.len().max(1);
    let current_line = line_index_at(&lines, snap.cursor) + 1;
    frame.render_widget(
        Paragraph::new(status_line(
            snap,
            current_line,
            total_lines,
            fx,
            show_live_speed,
        )),
        panes[2],
    );
}

fn status_line(
    snap: &TypingSnapshot,
    current_line: usize,
    total_lines: usize,
    fx: Option<&FxState>,
    show_live_speed: bool,
) -> Line<'static> {
    let sep = || Span::styled("  │  ", Style::default().fg(theme::BORDER));
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(
            format!("line {current_line}/{total_lines}"),
            Style::default().fg(theme::BLUE),
        ),
        sep(),
        Span::styled(
            format!("misses {}", snap.misses),
            Style::default().fg(if snap.misses > 0 {
                theme::RED
            } else {
                theme::MUTED
            }),
        ),
    ];
    if show_live_speed {
        let metrics = crate::domain::content::TypingMetrics::from_counts(
            snap.keystrokes,
            snap.misses,
            snap.elapsed_ms,
        );
        spans.push(sep());
        spans.push(Span::styled(
            format!("{:.0} kpm", metrics.kpm),
            Style::default().fg(theme::YELLOW),
        ));
    }
    if let Some(fx) = fx {
        let tier = fx.tier();
        let style = if tier > 0 {
            Style::default()
                .fg(tier_color(tier))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::MUTED)
        };
        spans.push(sep());
        spans.push(Span::styled(format!("⚡{}", fx.streak()), style));
    }
    spans.push(sep());
    spans.push(Span::styled(
        "Esc interrupt",
        Style::default().fg(theme::MUTED),
    ));
    Line::from(spans)
}

fn render_body(
    snap: &TypingSnapshot,
    syntax_colors: Option<&[Color]>,
    now_ms: u64,
    height: usize,
    fx: Option<&FxState>,
) -> Vec<Line<'static>> {
    let chars: Vec<char> = snap.target.chars().collect();
    let lines = split_lines(&chars);
    let cursor_line = line_index_at(&lines, snap.cursor);
    let flash = snap
        .miss_until_ms
        .map(|until| now_ms < until)
        .unwrap_or(false);
    let tier = fx.map(|f| f.tier()).unwrap_or(0);
    let preset = fx.map(|f| f.preset()).unwrap_or(FxPreset::Classic);
    let typed_auto = theme::lerp(theme::GREEN, theme::MUTED, 0.55);

    // Keep current line roughly centered.
    let half = height / 2;
    let start = cursor_line.saturating_sub(half);
    let end = (start + height).min(lines.len());

    let mut out = Vec::new();
    for (li, line) in lines.iter().enumerate().take(end).skip(start) {
        let is_cursor_line = li == cursor_line;
        let line_bg = if is_cursor_line {
            Some(theme::CURRENT_LINE_BG)
        } else {
            None
        };
        let trail = fx.and_then(|f| f.trail_at(li, now_ms));

        let mut spans = Vec::new();
        for (col, &(idx, ch)) in line.iter().enumerate() {
            let display = if ch == '\n' { '↵' } else { ch };
            let auto = snap.auto_inserted.binary_search(&idx).is_ok();
            let cursor_trail = fx.and_then(|state| state.cursor_trail_at(li, col, now_ms));
            let ripple = if preset == FxPreset::Ripple {
                fx.and_then(|state| state.ripple_at(li, col, now_ms))
            } else {
                None
            };
            let trail_color = match preset {
                FxPreset::Blaze => theme::ORANGE,
                FxPreset::Smear => theme::CYAN,
                FxPreset::Classic => theme::YELLOW,
                FxPreset::Ripple => theme::BLUE,
            };
            let bloom_color = match preset {
                FxPreset::Blaze => theme::ORANGE,
                FxPreset::Smear => theme::CYAN,
                FxPreset::Classic | FxPreset::Ripple => theme::BRIGHT,
            };

            let mut style = if idx < snap.cursor {
                let mut fg = syntax_colors
                    .and_then(|colors| colors.get(idx).copied())
                    .map(|color| theme::lerp(color, theme::BRIGHT, if auto { 0.08 } else { 0.2 }))
                    .unwrap_or(if auto { typed_auto } else { theme::GREEN });
                if let Some(glow) = fx.and_then(|f| f.glow_intensity(idx, now_ms)) {
                    let target = match preset {
                        FxPreset::Blaze => theme::ORANGE,
                        FxPreset::Smear => theme::CYAN,
                        FxPreset::Classic | FxPreset::Ripple => theme::BRIGHT,
                    };
                    let strength = match preset {
                        FxPreset::Blaze => (glow * 1.25 + tier as f32 * 0.05).clamp(0.0, 1.0),
                        _ => glow,
                    };
                    fg = theme::lerp(fg, target, strength);
                }
                let mut style = Style::default().fg(fg);
                if preset == FxPreset::Blaze
                    && fx
                        .and_then(|state| state.glow_intensity(idx, now_ms))
                        .is_some_and(|glow| glow > 0.35)
                {
                    style = style.add_modifier(Modifier::BOLD);
                }
                style
            } else if idx == snap.cursor {
                let mut bg = if flash { theme::RED } else { tier_color(tier) };
                if !flash {
                    if let Some(ripple) = ripple {
                        bg = theme::lerp(bg, theme::CYAN, ripple * 0.75);
                    }
                    if let Some(trail) = cursor_trail {
                        bg = theme::lerp(bg, trail_color, trail * 0.9);
                    }
                    if let Some(bloom) = fx.map(|state| state.cursor_bloom(now_ms)) {
                        bg = theme::lerp(bg, bloom_color, bloom * 0.95);
                    }
                }
                Style::default()
                    .fg(theme::BG)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                let mut fg = syntax_colors
                    .and_then(|colors| colors.get(idx).copied())
                    .map(|color| theme::lerp(color, theme::MUTED, 0.45))
                    .unwrap_or(theme::MUTED);
                if let Some(ripple) = ripple {
                    fg = theme::lerp(fg, theme::CYAN, ripple * 0.8);
                }
                if let Some(trail) = cursor_trail {
                    fg = theme::lerp(fg, trail_color, trail * 0.9);
                }
                Style::default().fg(fg)
            };

            if let Some(ripple) = ripple {
                let base = style.fg.unwrap_or(theme::MUTED);
                let ripple_color = if idx < snap.cursor {
                    theme::CYAN
                } else {
                    theme::BLUE
                };
                style = style.fg(theme::lerp(base, ripple_color, ripple));
                if ripple > 0.78 && idx != snap.cursor {
                    style = style.add_modifier(Modifier::BOLD);
                }
            }

            // Lightning-like sweep across a just-completed line.
            if let Some((progress, intensity)) = trail {
                let sweep_col = progress * line.len().max(1) as f32;
                let dist = (col as f32 - sweep_col).abs();
                let w = (1.0 - dist / 5.0).clamp(0.0, 1.0) * intensity;
                if w > 0.0 && idx != snap.cursor {
                    let base = style.fg.unwrap_or(theme::FG);
                    style = style.fg(theme::lerp(base, theme::CYAN, w));
                }
            }

            if idx != snap.cursor {
                if let Some(bg) = line_bg {
                    style = style.bg(bg);
                }
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
