//! Settings screen (home `c`): edit `config.toml` keys.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::config::UserConfig;
use crate::ui::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    Bool,
    TabWidth,
    ProgressMode,
    DependencyDirection,
    FxIntensity,
    FxPreset,
    /// Display-only / locked (cannot change).
    Locked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingId {
    IncludeImports,
    IncludeDocComments,
    IncludeComments,
    IncludeTests,
    IncludeConfigs,
    TabWidth,
    AutoIndent,
    AutoCloseBrackets,
    AllowBackspace,
    SyntaxHighlight,
    ShowLiveSpeed,
    ProgressMode,
    DependencyDirection,
    KeepDoneOnRefresh,
    HideSkipped,
    FxEnabled,
    FxIntensity,
    FxPreset,
}

#[derive(Debug, Clone, Copy)]
struct SettingDef {
    id: SettingId,
    section: &'static str,
    label: &'static str,
    kind: SettingKind,
    /// Shown muted; value still editable unless Locked.
    unsupported: bool,
}

const SETTINGS: &[SettingDef] = &[
    SettingDef {
        id: SettingId::IncludeImports,
        section: "content",
        label: "include_imports",
        kind: SettingKind::Bool,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::IncludeDocComments,
        section: "content",
        label: "include_doc_comments",
        kind: SettingKind::Bool,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::IncludeComments,
        section: "content",
        label: "include_comments",
        kind: SettingKind::Bool,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::IncludeTests,
        section: "content",
        label: "include_tests",
        kind: SettingKind::Bool,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::IncludeConfigs,
        section: "content",
        label: "include_configs",
        kind: SettingKind::Bool,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::TabWidth,
        section: "content",
        label: "tab_width",
        kind: SettingKind::TabWidth,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::AutoIndent,
        section: "typing",
        label: "auto_indent",
        kind: SettingKind::Bool,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::AutoCloseBrackets,
        section: "typing",
        label: "auto_close_brackets",
        kind: SettingKind::Locked,
        unsupported: true,
    },
    SettingDef {
        id: SettingId::AllowBackspace,
        section: "typing",
        label: "allow_backspace",
        kind: SettingKind::Bool,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::ShowLiveSpeed,
        section: "typing",
        label: "show_live_speed",
        kind: SettingKind::Bool,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::SyntaxHighlight,
        section: "typing",
        label: "syntax_highlight",
        kind: SettingKind::Bool,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::ProgressMode,
        section: "progress",
        label: "mode",
        kind: SettingKind::ProgressMode,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::DependencyDirection,
        section: "progress",
        label: "dependency_direction",
        kind: SettingKind::DependencyDirection,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::KeepDoneOnRefresh,
        section: "progress",
        label: "keep_done_on_refresh",
        kind: SettingKind::Bool,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::HideSkipped,
        section: "progress",
        label: "hide_skipped",
        kind: SettingKind::Bool,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::FxEnabled,
        section: "fx",
        label: "enabled",
        kind: SettingKind::Bool,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::FxIntensity,
        section: "fx",
        label: "intensity",
        kind: SettingKind::FxIntensity,
        unsupported: false,
    },
    SettingDef {
        id: SettingId::FxPreset,
        section: "fx",
        label: "preset",
        kind: SettingKind::FxPreset,
        unsupported: false,
    },
];

#[derive(Debug, Clone)]
pub struct SettingsView {
    pub selected: usize,
    pub status: Option<String>,
}

impl SettingsView {
    pub fn new() -> Self {
        Self {
            selected: 0,
            status: None,
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        let len = SETTINGS.len() as isize;
        let next = (self.selected as isize + delta).rem_euclid(len);
        self.selected = next as usize;
    }

    pub fn selected_id(&self) -> SettingId {
        SETTINGS[self.selected].id
    }

    pub fn selected_kind(&self) -> SettingKind {
        SETTINGS[self.selected].kind
    }

    /// Toggle / nudge the selected setting. Returns true if config changed.
    pub fn activate(&mut self, cfg: &mut UserConfig, forward: bool) -> bool {
        let def = SETTINGS[self.selected];
        if def.kind == SettingKind::Locked {
            self.status = Some("auto_close_brackets is not implemented yet".into());
            return false;
        }
        self.status = None;
        match def.id {
            SettingId::IncludeImports => {
                cfg.content.include_imports = !cfg.content.include_imports;
            }
            SettingId::IncludeDocComments => {
                cfg.content.include_doc_comments = !cfg.content.include_doc_comments;
            }
            SettingId::IncludeComments => {
                cfg.content.include_comments = !cfg.content.include_comments;
            }
            SettingId::IncludeTests => {
                cfg.content.include_tests = !cfg.content.include_tests;
            }
            SettingId::IncludeConfigs => {
                cfg.content.include_configs = !cfg.content.include_configs;
            }
            SettingId::TabWidth => {
                if forward {
                    cfg.content.tab_width = if cfg.content.tab_width >= 8 {
                        1
                    } else {
                        cfg.content.tab_width + 1
                    };
                } else {
                    cfg.content.tab_width = if cfg.content.tab_width <= 1 {
                        8
                    } else {
                        cfg.content.tab_width - 1
                    };
                }
            }
            SettingId::AutoIndent => {
                cfg.typing.auto_indent = !cfg.typing.auto_indent;
            }
            SettingId::AutoCloseBrackets => return false,
            SettingId::AllowBackspace => {
                cfg.typing.allow_backspace = !cfg.typing.allow_backspace;
            }
            SettingId::SyntaxHighlight => {
                cfg.typing.syntax_highlight = !cfg.typing.syntax_highlight;
            }
            SettingId::ShowLiveSpeed => {
                cfg.typing.show_live_speed = !cfg.typing.show_live_speed;
            }
            SettingId::ProgressMode => {
                cfg.progress.mode = cfg.progress.mode.cycle();
            }
            SettingId::DependencyDirection => {
                cfg.progress.dependency_direction = if forward {
                    cfg.progress.dependency_direction.cycle()
                } else {
                    // two-value cycle is symmetric
                    cfg.progress.dependency_direction.cycle()
                };
            }
            SettingId::KeepDoneOnRefresh => {
                cfg.progress.keep_done_on_refresh = !cfg.progress.keep_done_on_refresh;
            }
            SettingId::HideSkipped => {
                cfg.progress.hide_skipped = !cfg.progress.hide_skipped;
            }
            SettingId::FxEnabled => {
                cfg.fx.enabled = !cfg.fx.enabled;
            }
            SettingId::FxIntensity => {
                cfg.fx.intensity = if forward {
                    cfg.fx.intensity.cycle_next()
                } else {
                    cfg.fx.intensity.cycle_prev()
                };
            }
            SettingId::FxPreset => {
                cfg.fx.preset = if forward {
                    cfg.fx.preset.cycle_next()
                } else {
                    cfg.fx.preset.cycle_prev()
                };
            }
        }
        true
    }
}

impl Default for SettingsView {
    fn default() -> Self {
        Self::new()
    }
}

fn value_string(cfg: &UserConfig, id: SettingId) -> String {
    match id {
        SettingId::IncludeImports => cfg.content.include_imports.to_string(),
        SettingId::IncludeDocComments => cfg.content.include_doc_comments.to_string(),
        SettingId::IncludeComments => cfg.content.include_comments.to_string(),
        SettingId::IncludeTests => cfg.content.include_tests.to_string(),
        SettingId::IncludeConfigs => cfg.content.include_configs.to_string(),
        SettingId::TabWidth => cfg.content.tab_width.to_string(),
        SettingId::AutoIndent => cfg.typing.auto_indent.to_string(),
        SettingId::AutoCloseBrackets => "false".into(),
        SettingId::AllowBackspace => cfg.typing.allow_backspace.to_string(),
        SettingId::SyntaxHighlight => cfg.typing.syntax_highlight.to_string(),
        SettingId::ShowLiveSpeed => cfg.typing.show_live_speed.to_string(),
        SettingId::ProgressMode => cfg.progress.mode.as_str().into(),
        SettingId::DependencyDirection => cfg.progress.dependency_direction.as_str().into(),
        SettingId::KeepDoneOnRefresh => cfg.progress.keep_done_on_refresh.to_string(),
        SettingId::HideSkipped => cfg.progress.hide_skipped.to_string(),
        SettingId::FxEnabled => cfg.fx.enabled.to_string(),
        SettingId::FxIntensity => cfg.fx.intensity.as_str().into(),
        SettingId::FxPreset => cfg.fx.preset.as_str().into(),
    }
}

pub fn draw_settings(frame: &mut Frame, area: Rect, view: &SettingsView, cfg: &UserConfig) {
    theme::fill_background(frame, area);
    let block = theme::bordered_block(theme::title_line("Settings"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  ~/.config/repomonk/config.toml",
            Style::default().fg(theme::MUTED),
        ))),
        panes[0],
    );

    let mut items = Vec::new();
    let mut last_section = "";
    for def in SETTINGS {
        if def.section != last_section {
            items.push(ListItem::new(Line::from(Span::styled(
                format!("  [{}]", def.section),
                Style::default()
                    .fg(theme::BLUE)
                    .add_modifier(Modifier::BOLD),
            ))));
            last_section = def.section;
        }
        let value = value_string(cfg, def.id);
        let mut label = format!("    {:<24} {}", def.label, value);
        if def.unsupported {
            label.push_str("  (unsupported)");
        }
        if def.kind == SettingKind::Locked {
            label.push_str("  [locked]");
        }
        items.push(ListItem::new(Line::from(Span::styled(
            label,
            Style::default().fg(if def.unsupported || def.kind == SettingKind::Locked {
                theme::MUTED
            } else {
                theme::FG
            }),
        ))));
    }

    // Map selected setting index → list row (accounting for section headers).
    let list_index = {
        let mut row = 0usize;
        let mut last = "";
        for (i, def) in SETTINGS.iter().enumerate() {
            if def.section != last {
                row += 1;
                last = def.section;
            }
            if i == view.selected {
                break;
            }
            row += 1;
        }
        row
    };

    let list = List::new(items).highlight_style(
        Style::default()
            .bg(theme::CURRENT_LINE_BG)
            .fg(theme::FG)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default();
    state.select(Some(list_index));
    frame.render_stateful_widget(list, panes[1], &mut state);

    let status = view
        .status
        .as_deref()
        .unwrap_or("Changes apply on next open / typing session. Content filters need re-open.");
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {status}"),
            Style::default().fg(theme::MUTED),
        ))),
        panes[2],
    );

    frame.render_widget(
        Paragraph::new(theme::key_hints(&[
            ("j/k", "move"),
            ("Enter/Space", "toggle"),
            ("h/l", "adjust"),
            ("Esc", "back"),
            ("q", "quit"),
        ])),
        panes[3],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FxPreset;

    #[test]
    fn fx_preset_setting_cycles_through_all_presets() {
        let mut view = SettingsView::new();
        view.selected = SETTINGS
            .iter()
            .position(|def| def.id == SettingId::FxPreset)
            .expect("fx preset setting");
        let mut cfg = UserConfig::default();

        for expected in [
            FxPreset::Blaze,
            FxPreset::Smear,
            FxPreset::Ripple,
            FxPreset::Classic,
        ] {
            assert!(view.activate(&mut cfg, true));
            assert_eq!(cfg.fx.preset, expected);
        }
    }

    #[test]
    fn syntax_highlight_setting_toggles() {
        let mut view = SettingsView::new();
        view.selected = SETTINGS
            .iter()
            .position(|def| def.id == SettingId::SyntaxHighlight)
            .expect("syntax highlight setting");
        let mut cfg = UserConfig::default();

        assert!(cfg.typing.syntax_highlight);
        assert!(view.activate(&mut cfg, true));
        assert!(!cfg.typing.syntax_highlight);
        assert!(view.activate(&mut cfg, true));
        assert!(cfg.typing.syntax_highlight);
    }
}
