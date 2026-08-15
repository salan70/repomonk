//! Help overlay: current-place keys, common keys, and tree legend.
//!
//! Rendered as a floating dialog over the current place, the same way File types floats.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::ui::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpContext {
    Home,
    Tree,
    Typing,
    Result,
    Search,
    Settings,
    Stats,
    Pause,
    FileTypes,
    Flow,
}

pub fn draw_help(frame: &mut Frame, area: Rect, ctx: HelpContext) {
    let card_height = match ctx {
        HelpContext::Tree => 32,
        HelpContext::Home => 26,
        _ => 20,
    };
    let card = theme::centered_rect(area, area.width.saturating_sub(8).min(72), card_height);
    frame.render_widget(Clear, card);
    let block = theme::bordered_block(theme::title_line("Help"));
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let mut lines = vec![section("here")];
    lines.extend(context_lines(ctx));
    if !matches!(ctx, HelpContext::Typing | HelpContext::Pause) {
        lines.push(Line::from(""));
        lines.push(section("common"));
        lines.extend([
            hint("?", "help"),
            hint(",", "settings"),
            hint("S", "stats"),
            hint("Esc/q", "back / close"),
        ]);
    }
    if matches!(ctx, HelpContext::Home | HelpContext::Tree) {
        lines.push(Line::from(""));
        lines.push(section("legend"));
        lines.extend([
            hint("✓", "done"),
            hint("○", "todo"),
            hint("·", "skipped"),
            hint("█░", "dir progress"),
            hint("— reason", "why skipped"),
            hint("▸ Next", "next file"),
            hint("—", "outside flow"),
            hint("123", "flow order"),
        ]);
    }
    lines.push(Line::from(""));
    lines.push(theme::key_hints(&[("Esc", "close"), ("?", "close")]));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn section(title: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {title}"),
        Style::default()
            .fg(theme::BLUE)
            .add_modifier(Modifier::BOLD),
    ))
}

fn hint(key: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {key:<14}"),
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc.to_string(), Style::default().fg(theme::MUTED)),
    ])
}

fn context_lines(ctx: HelpContext) -> Vec<Line<'static>> {
    match ctx {
        HelpContext::Home => vec![
            hint("Enter", "open"),
            hint("j/k", "select"),
            hint("g/G", "first / last"),
            hint("/", "search"),
            hint("q", "quit"),
        ],
        HelpContext::Tree => vec![
            hint("Enter", "open"),
            hint("j/k", "move"),
            hint("g/G", "first / last"),
            hint("Ctrl-d/u", "half page"),
            hint("Tab", "next file"),
            hint("/", "filter"),
            hint("x/X", "skip / reset"),
            hint("h/l", "fold"),
            hint("e", "how to proceed"),
            hint("t", "file types"),
        ],
        HelpContext::Typing => vec![
            hint("keys", "type the source"),
            hint("Esc", "pause"),
            hint("Ctrl-C", "quit"),
        ],
        HelpContext::Result => vec![
            hint("Enter", "next"),
            hint("r", "retry"),
            hint("t/Esc", "tree"),
        ],
        HelpContext::Search => vec![
            hint("Enter", "open"),
            hint("↑/↓", "select"),
            hint("Ctrl-n/p", "select"),
            hint("Esc", "close"),
        ],
        HelpContext::Settings => vec![
            hint("j/k", "move"),
            hint("h/l", "adjust"),
            hint("Enter", "toggle"),
            hint("Esc", "close"),
        ],
        HelpContext::Stats => vec![
            hint("j/k", "scroll"),
            hint("g/G", "first / last"),
            hint("Esc", "close"),
        ],
        HelpContext::Pause => vec![
            hint("Esc/Enter", "resume"),
            hint("r", "retry"),
            hint("t", "tree"),
        ],
        HelpContext::FileTypes => vec![
            hint("j/k", "move"),
            hint("Space/Enter", "cycle: on / off / hide"),
            hint("Esc/q/t", "apply & close"),
        ],
        HelpContext::Flow => vec![
            hint("j/k", "move"),
            hint("Enter/Space", "select"),
            hint("Esc/q/e", "apply & close"),
        ],
    }
}
