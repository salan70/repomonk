//! Help overlay: current-place keys, common keys, and tree legend.
//!
//! Rendered as a floating dialog over the current place, the same way File types floats.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::ui::i18n::UiStrings;
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

pub fn draw_help(frame: &mut Frame, area: Rect, ctx: HelpContext, t: &UiStrings) {
    // Home and Tree both carry the legend, so the extra `◐` row costs each one line.
    let card_height = match ctx {
        HelpContext::Tree => 35,
        HelpContext::Home => 27,
        _ => 20,
    };
    let card = theme::centered_rect(area, area.width.saturating_sub(8).min(72), card_height);
    frame.render_widget(Clear, card);
    let block = theme::bordered_block(theme::title_line(t.title_help));
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let mut lines = vec![section(t.section_here)];
    lines.extend(context_lines(ctx, t));
    if !matches!(ctx, HelpContext::Typing | HelpContext::Pause) {
        lines.push(Line::from(""));
        lines.push(section(t.section_common));
        lines.extend([
            hint("?", t.help),
            hint(",", t.settings),
            hint("S", t.stats),
            hint("Esc/q", t.back_close),
        ]);
    }
    if matches!(ctx, HelpContext::Home | HelpContext::Tree) {
        lines.push(Line::from(""));
        lines.push(section(t.section_legend));
        lines.extend([
            hint("✓", t.legend_done),
            hint("◐", t.legend_in_progress),
            hint("○", t.legend_todo),
            hint("·", t.legend_skipped),
            hint("█░", t.legend_dir_progress),
            hint("— reason", t.legend_why_skipped),
            hint("▸ Next", t.legend_next_file),
            hint("—", t.legend_outside_flow),
            hint("123", t.legend_flow_order),
        ]);
    }
    lines.push(Line::from(""));
    lines.push(theme::key_hints(&[("Esc", t.close), ("?", t.close)]));

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

fn context_lines(ctx: HelpContext, t: &UiStrings) -> Vec<Line<'static>> {
    match ctx {
        HelpContext::Home => vec![
            hint("Enter", t.open),
            hint("j/k", t.select),
            hint("g/G", t.first_last),
            hint("/", t.search),
            hint("q", t.quit),
        ],
        HelpContext::Tree => vec![
            hint("Enter", t.open),
            hint("j/k", t.move_),
            hint("g/G", t.first_last),
            hint("Ctrl-d/u", t.half_page),
            hint("Tab", t.next_file),
            hint("/", t.filter),
            hint("x/X", t.skip_reset),
            hint(".", t.show_hide_excluded),
            hint("h/l", t.fold),
            hint("o", t.expand_or_collapse),
            hint("e", t.how_to_proceed),
            hint("t", t.file_types),
        ],
        HelpContext::Typing => vec![
            hint("keys", t.type_source),
            hint("Esc", t.pause),
            hint("Ctrl-C", t.quit),
        ],
        HelpContext::Result => vec![
            hint("Enter", t.next),
            hint("r", t.retry),
            hint("t/Esc", t.tree),
        ],
        HelpContext::Search => vec![
            hint("Enter", t.open),
            hint("↑/↓", t.select),
            hint("Ctrl-n/p", t.select),
            hint("Esc", t.close),
        ],
        HelpContext::Settings => vec![
            hint("j/k", t.move_),
            hint("h/l", t.adjust),
            hint("Enter", t.toggle),
            hint("Esc", t.close),
        ],
        HelpContext::Stats => vec![
            hint("j/k", t.scroll),
            hint("g/G", t.first_last),
            hint("Esc", t.close),
        ],
        HelpContext::Pause => vec![
            hint("Esc/Enter", t.resume),
            hint("r", t.retry),
            hint("t", t.tree),
        ],
        HelpContext::FileTypes => vec![
            hint("j/k", t.move_),
            hint("Space/Enter", t.cycle_on_off_hide),
            hint("Esc/q/t", t.apply_close),
        ],
        HelpContext::Flow => vec![
            hint("j/k", t.move_),
            hint("Enter/Space", t.select),
            hint("Esc/q/e", t.apply_close),
        ],
    }
}
