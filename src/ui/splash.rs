//! Startup splash: ASCII logo reveal with a glow sweep.
//!
//! The animation timeline is a pure function of elapsed milliseconds
//! ([`splash_frame`]) so it can be unit-tested without a terminal.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::ui::theme;

pub const LOGO: [&str; 6] = [
    r"                                                    _    ",
    r" _ __   ___  _ __    ___   _ __ ___    ___   _ __  | | __",
    r"| '__| / _ \| '_ \  / _ \ | '_ ` _ \  / _ \ | '_ \ | |/ /",
    r"| |   |  __/| |_) || (_) || | | | | || (_) || | | ||   < ",
    r"|_|    \___|| .__/  \___/ |_| |_| |_| \___/ |_| |_||_|\_\",
    r"            |_|                                          ",
];

/// Milliseconds between each revealed logo row.
pub const REVEAL_ROW_MS: u64 = 80;
/// Glow sweep window (after the logo is fully revealed).
pub const GLOW_START_MS: u64 = REVEAL_ROW_MS * LOGO.len() as u64;
pub const GLOW_DURATION_MS: u64 = 800;
/// Tagline fade-in window.
pub const TAGLINE_START_MS: u64 = 700;
pub const TAGLINE_DURATION_MS: u64 = 400;
/// Total splash duration before auto-advancing to the tree.
pub const SPLASH_TOTAL_MS: u64 = 2_000;

/// Pure description of the splash at a given elapsed time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplashFrame {
    /// Number of logo rows revealed (0..=LOGO.len()).
    pub visible_rows: usize,
    /// Glow sweep phase across the logo width (0..=1).
    pub glow_phase: f32,
    /// Tagline opacity (0..=1).
    pub tagline_alpha: f32,
    /// Whether the splash should transition to the tree.
    pub done: bool,
}

pub fn splash_frame(elapsed_ms: u64) -> SplashFrame {
    let visible_rows = ((elapsed_ms / REVEAL_ROW_MS) as usize + 1).min(LOGO.len());
    let glow_phase = if elapsed_ms <= GLOW_START_MS {
        0.0
    } else {
        ((elapsed_ms - GLOW_START_MS) as f32 / GLOW_DURATION_MS as f32).clamp(0.0, 1.0)
    };
    let tagline_alpha = if elapsed_ms <= TAGLINE_START_MS {
        0.0
    } else {
        ((elapsed_ms - TAGLINE_START_MS) as f32 / TAGLINE_DURATION_MS as f32).clamp(0.0, 1.0)
    };
    SplashFrame {
        visible_rows,
        glow_phase,
        tagline_alpha,
        done: elapsed_ms >= SPLASH_TOTAL_MS,
    }
}

/// Maximum logo width in characters (all rows share the same width).
pub fn logo_width() -> usize {
    LOGO.iter().map(|l| l.chars().count()).max().unwrap_or(0)
}

/// Animated ASCII logo lines (reveal + glow), without tagline.
pub fn logo_lines(elapsed_ms: u64) -> Vec<Line<'static>> {
    let state = splash_frame(elapsed_ms);
    let width = logo_width();
    let mut lines: Vec<Line> = Vec::with_capacity(LOGO.len());
    for (row, art) in LOGO.iter().enumerate() {
        if row >= state.visible_rows {
            lines.push(Line::from(""));
            continue;
        }
        let just_revealed = row + 1 == state.visible_rows && state.visible_rows < LOGO.len();
        let spans: Vec<Span> = art
            .chars()
            .enumerate()
            .map(|(col, ch)| {
                let base = gradient_color(col, width);
                let mut color = base;
                if just_revealed {
                    color = theme::BRIGHT;
                } else if state.glow_phase > 0.0 && state.glow_phase < 1.0 {
                    let sweep_col = state.glow_phase * width as f32;
                    let dist = (col as f32 - sweep_col).abs();
                    let w = (1.0 - dist / 8.0).clamp(0.0, 1.0);
                    color = theme::lerp(base, theme::BRIGHT, w);
                }
                Span::styled(
                    ch.to_string(),
                    Style::default()
                        .fg(color)
                        .bg(theme::BG)
                        .add_modifier(Modifier::BOLD),
                )
            })
            .collect();
        lines.push(Line::from(spans));
    }
    lines
}

/// Draw the animated logo centered in `area` (logo only).
pub fn draw_animated_logo(frame: &mut Frame, area: Rect, elapsed_ms: u64) {
    let width = logo_width() as u16;
    let target = theme::centered_rect(area, width.min(area.width), LOGO.len() as u16);
    frame.render_widget(Paragraph::new(logo_lines(elapsed_ms)), target);
}

pub fn draw_splash(frame: &mut Frame, area: Rect, elapsed_ms: u64, repo_name: &str) {
    theme::fill_background(frame, area);
    let state = splash_frame(elapsed_ms);

    let width = logo_width();
    let height = LOGO.len() as u16 + 3;
    let target = theme::centered_rect(area, width as u16, height);

    let mut lines = logo_lines(elapsed_ms);
    lines.push(Line::from(""));
    let tagline = format!("v{}  —  {}", env!("CARGO_PKG_VERSION"), repo_name);
    let pad = width.saturating_sub(tagline.chars().count()) / 2;
    lines.push(Line::from(Span::styled(
        format!("{}{}", " ".repeat(pad), tagline),
        Style::default()
            .fg(theme::lerp(theme::BG, theme::MUTED, state.tagline_alpha))
            .bg(theme::BG),
    )));
    let hint = "press any key";
    let pad = width.saturating_sub(hint.chars().count()) / 2;
    lines.push(Line::from(Span::styled(
        format!("{}{}", " ".repeat(pad), hint),
        Style::default()
            .fg(theme::lerp(theme::BG, theme::BORDER, state.tagline_alpha))
            .bg(theme::BG),
    )));

    frame.render_widget(Paragraph::new(lines), target);
}

/// Horizontal magenta → blue → cyan gradient across the logo.
fn gradient_color(col: usize, width: usize) -> ratatui::style::Color {
    let t = if width <= 1 {
        0.0
    } else {
        col as f32 / (width - 1) as f32
    };
    if t < 0.5 {
        theme::lerp(theme::MAGENTA, theme::BLUE, t * 2.0)
    } else {
        theme::lerp(theme::BLUE, theme::CYAN, (t - 0.5) * 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_progresses_row_by_row() {
        assert_eq!(splash_frame(0).visible_rows, 1);
        assert_eq!(splash_frame(REVEAL_ROW_MS).visible_rows, 2);
        assert_eq!(
            splash_frame(REVEAL_ROW_MS * LOGO.len() as u64).visible_rows,
            LOGO.len()
        );
        assert_eq!(splash_frame(10_000).visible_rows, LOGO.len());
    }

    #[test]
    fn glow_starts_after_reveal_and_saturates() {
        assert_eq!(splash_frame(GLOW_START_MS).glow_phase, 0.0);
        let mid = splash_frame(GLOW_START_MS + GLOW_DURATION_MS / 2).glow_phase;
        assert!(mid > 0.4 && mid < 0.6);
        assert_eq!(
            splash_frame(GLOW_START_MS + GLOW_DURATION_MS).glow_phase,
            1.0
        );
    }

    #[test]
    fn tagline_fades_in() {
        assert_eq!(splash_frame(TAGLINE_START_MS).tagline_alpha, 0.0);
        assert_eq!(
            splash_frame(TAGLINE_START_MS + TAGLINE_DURATION_MS).tagline_alpha,
            1.0
        );
    }

    #[test]
    fn done_after_total_duration() {
        assert!(!splash_frame(SPLASH_TOTAL_MS - 1).done);
        assert!(splash_frame(SPLASH_TOTAL_MS).done);
    }

    #[test]
    fn logo_rows_have_consistent_width() {
        let widths: Vec<usize> = LOGO.iter().map(|l| l.chars().count()).collect();
        assert!(widths.iter().all(|&w| w == widths[0]), "widths: {widths:?}");
    }
}
