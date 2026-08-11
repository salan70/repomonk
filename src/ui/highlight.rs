//! Syntax highlighting for the immutable source shown on the typing screen.
//!
//! Highlighting is deliberately kept in the UI layer. It changes how the
//! normalized source is rendered, but never changes typing validation or
//! progress accounting.

use std::str::FromStr;
use std::sync::OnceLock;

use ratatui::style::Color;
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color as SyntectColor, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings,
};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

use crate::ui::theme;

/// Return one syntax color for every Unicode scalar value in `text`.
///
/// The result is aligned with `TypingSnapshot::target.chars()`, not with the
/// UTF-8 byte offsets used internally by syntect. Unsupported extensions and
/// parser failures intentionally fall back to the normal foreground color so
/// typing can continue without a rendering error.
pub fn highlight_chars(relative_path: &str, text: &str) -> Vec<Color> {
    let char_count = text.chars().count();
    let fallback = vec![theme::FG; char_count];
    let syntax = syntax_for_path(relative_path, syntax_set());
    let Some(syntax) = syntax else {
        return fallback;
    };

    let mut highlighter = HighlightLines::new(syntax, highlight_theme());
    let mut colors = vec![theme::FG; char_count];
    let mut char_index = 0usize;

    for line in LinesWithEndings::from(text) {
        let ranges = match highlighter.highlight_line(line, syntax_set()) {
            Ok(ranges) => ranges,
            Err(_) => return fallback,
        };

        for (style, token) in ranges {
            let token_len = token.chars().count();
            let end = char_index.saturating_add(token_len);
            if end > colors.len() {
                return fallback;
            }
            let color = to_ratatui_color(style.foreground);
            colors[char_index..end].fill(color);
            char_index = end;
        }
    }

    if char_index == char_count {
        colors
    } else {
        fallback
    }
}

fn syntax_set() -> &'static SyntaxSet {
    static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn syntax_for_path<'a>(
    relative_path: &str,
    syntax_set: &'a SyntaxSet,
) -> Option<&'a SyntaxReference> {
    let extension = relative_path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())?;
    syntax_set.find_syntax_by_extension(&extension).or_else(|| {
        let fallback_extension = match extension.as_str() {
            "cjs" | "jsx" | "mjs" | "ts" | "tsx" => Some("js"),
            "pyi" => Some("py"),
            _ => None,
        }?;
        syntax_set.find_syntax_by_extension(fallback_extension)
    })
}

fn highlight_theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(|| {
        let mut theme = Theme {
            name: Some("repomonk Tokyo Night".into()),
            author: Some("repomonk".into()),
            settings: ThemeSettings {
                foreground: Some(to_syntect_color(theme::FG)),
                background: Some(to_syntect_color(theme::BG)),
                ..ThemeSettings::default()
            },
            scopes: Vec::new(),
        };

        // The selectors are TextMate scopes emitted by the bundled grammars.
        // Keep the palette small and aligned with the rest of the TUI.
        for (scope, color) in [
            ("comment", theme::MUTED),
            ("string", theme::GREEN),
            ("constant.character", theme::GREEN),
            ("constant.numeric", theme::ORANGE),
            ("constant.language", theme::MAGENTA),
            ("keyword", theme::MAGENTA),
            ("storage", theme::MAGENTA),
            ("entity.name.function", theme::BLUE),
            ("support.function", theme::BLUE),
            ("entity.name.type", theme::CYAN),
            ("support.type", theme::CYAN),
            ("variable.parameter", theme::YELLOW),
            ("invalid", theme::RED),
        ] {
            theme.scopes.push(ThemeItem {
                scope: ScopeSelectors::from_str(scope)
                    .expect("built-in syntax highlight selector must be valid"),
                style: StyleModifier {
                    foreground: Some(to_syntect_color(color)),
                    ..StyleModifier::default()
                },
            });
        }

        theme
    })
}

fn to_syntect_color(color: Color) -> SyntectColor {
    match color {
        Color::Rgb(r, g, b) => SyntectColor { r, g, b, a: 0xff },
        _ => SyntectColor {
            r: 0xc0,
            g: 0xca,
            b: 0xf5,
            a: 0xff,
        },
    }
}

fn to_ratatui_color(color: SyntectColor) -> Color {
    Color::Rgb(color.r, color.g, color.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_rust_keywords_and_functions_differently() {
        let source = "fn main() {\n    let answer = \"ok\";\n}\n";
        let colors = highlight_chars("src/main.rs", source);

        assert_eq!(colors.len(), source.chars().count());
        assert_eq!(colors[0], theme::MAGENTA);
        assert_eq!(colors[3], theme::BLUE);
        let string_index = source.chars().position(|ch| ch == '"').unwrap();
        assert_eq!(colors[string_index], theme::GREEN);
    }

    #[test]
    fn comments_use_muted_color() {
        let source = "// keep this local\n";
        let colors = highlight_chars("main.rs", source);

        assert!(colors.iter().all(|color| *color == theme::MUTED));
    }

    #[test]
    fn highlights_typescript_tokens() {
        let source = "const answer = \"ok\";\n";
        let colors = highlight_chars("src/main.ts", source);

        assert_eq!(colors[0], theme::MAGENTA);
        let string_index = source.chars().position(|ch| ch == '"').unwrap();
        assert_eq!(colors[string_index], theme::GREEN);
    }

    #[test]
    fn unsupported_extensions_fall_back_to_foreground() {
        let source = "fn main() {}";
        let colors = highlight_chars("README.unknown", source);

        assert_eq!(colors, vec![theme::FG; source.chars().count()]);
    }

    #[test]
    fn unicode_alignment_is_by_character() {
        let source = "let greeting = \"日本語\";\n";
        let colors = highlight_chars("main.rs", source);

        assert_eq!(colors.len(), source.chars().count());
        assert_eq!(
            colors[source.chars().position(|ch| ch == '日').unwrap()],
            theme::GREEN
        );
    }
}
