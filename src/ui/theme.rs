//! Tokyo Night palette and shared styling helpers.
//!
//! All screens draw against this palette instead of terminal defaults, so the
//! app looks the same regardless of the user's terminal color scheme
//! (truecolor support is assumed).

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};
use ratatui::Frame;

// Tokyo Night (night variant) palette.
pub const BG: Color = Color::Rgb(0x1a, 0x1b, 0x26);
pub const FG: Color = Color::Rgb(0xc0, 0xca, 0xf5);
pub const BLUE: Color = Color::Rgb(0x7a, 0xa2, 0xf7);
pub const CYAN: Color = Color::Rgb(0x7d, 0xcf, 0xff);
pub const GREEN: Color = Color::Rgb(0x9e, 0xce, 0x6a);
pub const YELLOW: Color = Color::Rgb(0xe0, 0xaf, 0x68);
pub const RED: Color = Color::Rgb(0xf7, 0x76, 0x8e);
pub const MAGENTA: Color = Color::Rgb(0xbb, 0x9a, 0xf7);
pub const ORANGE: Color = Color::Rgb(0xff, 0x9e, 0x64);
pub const MUTED: Color = Color::Rgb(0x56, 0x5f, 0x89);
pub const BORDER: Color = Color::Rgb(0x41, 0x48, 0x68);
pub const SELECTION_BG: Color = BORDER;
pub const CURRENT_LINE_BG: Color = Color::Rgb(0x29, 0x2e, 0x42);
/// Brightened foreground used for glow highlights.
pub const BRIGHT: Color = Color::Rgb(0xe6, 0xed, 0xff);

pub fn base_style() -> Style {
    Style::default().fg(FG).bg(BG)
}

/// Paint the whole area with the theme background.
pub fn fill_background(frame: &mut Frame, area: Rect) {
    frame.render_widget(Block::default().style(base_style()), area);
}

/// Rounded bordered block with a themed title line.
pub fn bordered_block(title: Line<'static>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER).bg(BG))
        .style(base_style())
        .title(title)
}

/// Screen title: accent-colored, bold.
pub fn title_line(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {text} "),
        Style::default()
            .fg(BLUE)
            .bg(BG)
            .add_modifier(Modifier::BOLD),
    ))
}

/// Key-hint footer: keys in cyan, descriptions muted, dot separators.
pub fn key_hints(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    spans.push(Span::styled(" ", base_style()));
    for (i, (key, desc)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ·  ", Style::default().fg(MUTED).bg(BG)));
        }
        spans.push(Span::styled(
            (*key).to_string(),
            Style::default()
                .fg(CYAN)
                .bg(BG)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {desc}"),
            Style::default().fg(MUTED).bg(BG),
        ));
    }
    Line::from(spans)
}

/// Linear interpolation between two colors (`t` clamped to 0..=1).
pub fn lerp(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let (ar, ag, ab) = rgb(a);
    let (br, bg_, bb) = rgb(b);
    Color::Rgb(mix(ar, br, t), mix(ag, bg_, t), mix(ab, bb, t))
}

fn rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (0xc0, 0xca, 0xf5),
    }
}

fn mix(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

/// Centered sub-rectangle of at most `width` x `height`.
pub fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}
