//! User configuration (`~/.config/repomonk/config.toml`).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;

/// Top-level user settings matching product-requirements §11.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserConfig {
    #[serde(default)]
    pub ui: UiConfig,
    pub content: ContentConfig,
    pub typing: TypingConfig,
    pub progress: ProgressConfig,
    pub fx: FxConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub language: UiLanguage,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            language: UiLanguage::Ja,
        }
    }
}

/// Display language for the TUI and user-facing messages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiLanguage {
    #[default]
    Ja,
    En,
}

impl UiLanguage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ja => "ja",
            Self::En => "en",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Ja => Self::En,
            Self::En => Self::Ja,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentConfig {
    pub include_imports: bool,
    pub include_doc_comments: bool,
    pub include_comments: bool,
    pub include_tests: bool,
    pub include_configs: bool,
    pub tab_width: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypingConfig {
    pub auto_indent: bool,
    pub auto_close_brackets: bool,
    pub allow_backspace: bool,
    #[serde(default = "default_syntax_highlight")]
    pub syntax_highlight: bool,
    pub show_live_speed: bool,
}

fn default_syntax_highlight() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressConfig {
    pub mode: ProgressMode,
    pub dependency_direction: DependencyDirection,
    pub keep_done_on_refresh: bool,
    pub hide_skipped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FxConfig {
    pub enabled: bool,
    pub intensity: FxIntensity,
    #[serde(default)]
    pub preset: FxPreset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressMode {
    Manual,
    #[serde(alias = "dependency")]
    Flow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyDirection {
    TopDown,
    BottomUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FxIntensity {
    Off,
    Subtle,
    Normal,
    High,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FxPreset {
    #[default]
    Classic,
    #[serde(alias = "flare", alias = "ember")]
    Blaze,
    #[serde(alias = "comet", alias = "matrix")]
    Smear,
    #[serde(alias = "pulse")]
    Ripple,
    Fireworks,
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            ui: UiConfig::default(),
            content: ContentConfig {
                include_imports: false,
                include_doc_comments: true,
                include_comments: false,
                include_tests: false,
                include_configs: false,
                tab_width: 4,
            },
            typing: TypingConfig {
                auto_indent: true,
                auto_close_brackets: false,
                allow_backspace: true,
                syntax_highlight: true,
                show_live_speed: false,
            },
            progress: ProgressConfig {
                mode: ProgressMode::Manual,
                dependency_direction: DependencyDirection::TopDown,
                keep_done_on_refresh: true,
                hide_skipped: true,
            },
            fx: FxConfig {
                enabled: true,
                intensity: FxIntensity::Normal,
                preset: FxPreset::Classic,
            },
        }
    }
}

impl ProgressMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Flow => "flow",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "manual" => Some(Self::Manual),
            "flow" | "dependency" => Some(Self::Flow),
            _ => None,
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Manual => Self::Flow,
            Self::Flow => Self::Manual,
        }
    }
}

impl DependencyDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TopDown => "top_down",
            Self::BottomUp => "bottom_up",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::TopDown => Self::BottomUp,
            Self::BottomUp => Self::TopDown,
        }
    }
}

impl FxIntensity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Subtle => "subtle",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }

    pub fn cycle_next(self) -> Self {
        match self {
            Self::Off => Self::Subtle,
            Self::Subtle => Self::Normal,
            Self::Normal => Self::High,
            Self::High => Self::Off,
        }
    }

    pub fn cycle_prev(self) -> Self {
        match self {
            Self::Off => Self::High,
            Self::Subtle => Self::Off,
            Self::Normal => Self::Subtle,
            Self::High => Self::Normal,
        }
    }

    /// Multiplier for glow / trail lifetimes (and similar).
    pub fn lifetime_scale(self) -> f32 {
        match self {
            Self::Off => 0.0,
            Self::Subtle => 0.6,
            Self::Normal => 1.0,
            Self::High => 1.5,
        }
    }
}

impl FxPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Blaze => "blaze",
            Self::Smear => "smear",
            Self::Ripple => "ripple",
            Self::Fireworks => "fireworks",
        }
    }

    pub fn cycle_next(self) -> Self {
        match self {
            Self::Classic => Self::Blaze,
            Self::Blaze => Self::Smear,
            Self::Smear => Self::Ripple,
            Self::Ripple => Self::Fireworks,
            Self::Fireworks => Self::Classic,
        }
    }

    pub fn cycle_prev(self) -> Self {
        match self {
            Self::Classic => Self::Fireworks,
            Self::Blaze => Self::Classic,
            Self::Smear => Self::Blaze,
            Self::Ripple => Self::Smear,
            Self::Fireworks => Self::Ripple,
        }
    }
}

impl UserConfig {
    /// Whether visual effects should run (ignores CLI `--no-fx`).
    pub fn fx_active(&self) -> bool {
        self.fx.enabled && self.fx.intensity != FxIntensity::Off
    }
}

/// Default path: `$XDG_CONFIG_HOME/repomonk/config.toml` (or platform equivalent).
pub fn default_config_path() -> crate::Result<PathBuf> {
    let base = dirs::config_dir().ok_or_else(|| {
        Error::Config(
            "could not resolve config directory; set XDG_CONFIG_HOME or create ~/.config".into(),
        )
    })?;
    Ok(base.join("repomonk").join("config.toml"))
}

/// Load config from `path`. Missing file returns defaults.
pub fn load(path: &Path) -> crate::Result<UserConfig> {
    if !path.exists() {
        return Ok(UserConfig::default());
    }
    let text = fs::read_to_string(path).map_err(|e| {
        Error::Config(format!(
            "failed to read {}: {e}\nFix: ensure the file is readable",
            path.display()
        ))
    })?;
    parse_toml(&text, path)
}

fn parse_toml(text: &str, path: &Path) -> crate::Result<UserConfig> {
    let cfg: UserConfig = toml::from_str(text).map_err(|e| {
        Error::Config(format!(
            "invalid config at {}:\n  {e}\nFix: correct the value to match docs/product-requirements.md §11 \
             (booleans true/false, tab_width 1–8, mode \"manual\"|\"flow\", \
             dependency_direction \"top_down\"|\"bottom_up\", intensity \"off\"|\"subtle\"|\"normal\"|\"high\", \
             preset \"classic\"|\"blaze\"|\"smear\"|\"ripple\"|\"fireworks\", \
             language \"ja\"|\"en\")",
            path.display()
        ))
    })?;
    validate(&cfg, path)?;
    Ok(cfg)
}

fn validate(cfg: &UserConfig, path: &Path) -> crate::Result<()> {
    if !(1..=8).contains(&cfg.content.tab_width) {
        return Err(Error::Config(format!(
            "invalid config at {}:\n  content.tab_width = {} is out of range\n\
             Fix: set content.tab_width to an integer between 1 and 8",
            path.display(),
            cfg.content.tab_width
        )));
    }
    if cfg.typing.auto_close_brackets {
        return Err(Error::Config(format!(
            "invalid config at {}:\n  typing.auto_close_brackets = true is not supported yet\n\
             Fix: set typing.auto_close_brackets = false",
            path.display()
        )));
    }
    Ok(())
}

/// Persist config, creating parent directories as needed.
pub fn save(path: &Path, cfg: &UserConfig) -> crate::Result<()> {
    validate(cfg, path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| Error::Config(format!("failed to create {}: {e}", parent.display())))?;
    }
    let text = toml::to_string_pretty(cfg)
        .map_err(|e| Error::Config(format!("failed to serialize config: {e}")))?;
    let header = "# repomonk user configuration\n# See docs/product-requirements.md §11\n\n";
    fs::write(path, format!("{header}{text}"))
        .map_err(|e| Error::Config(format!("failed to write {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing.toml");
        let cfg = load(&path).unwrap();
        assert_eq!(cfg, UserConfig::default());
    }

    #[test]
    fn round_trip_save_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut cfg = UserConfig::default();
        cfg.content.tab_width = 2;
        cfg.typing.syntax_highlight = false;
        cfg.typing.show_live_speed = true;
        cfg.fx.intensity = FxIntensity::Subtle;
        cfg.fx.preset = FxPreset::Fireworks;
        save(&path, &cfg).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.content.tab_width, 2);
        assert!(!loaded.typing.syntax_highlight);
        assert!(loaded.typing.show_live_speed);
        assert_eq!(loaded.fx.intensity, FxIntensity::Subtle);
        assert_eq!(loaded.fx.preset, FxPreset::Fireworks);
    }

    #[test]
    fn missing_ui_section_defaults_to_japanese() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut text = toml::to_string_pretty(&UserConfig::default()).unwrap();
        if let Some(idx) = text.find("[ui]") {
            let rest = &text[idx + 4..];
            let next = rest.find("\n[").map(|i| idx + 4 + i).unwrap_or(text.len());
            text.replace_range(idx..next, "");
        }
        fs::write(&path, text).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.ui.language, UiLanguage::Ja);
    }

    #[test]
    fn language_en_round_trips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut cfg = UserConfig::default();
        cfg.ui.language = UiLanguage::En;
        save(&path, &cfg).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.ui.language, UiLanguage::En);
    }

    #[test]
    fn presets_cycle_through_fireworks() {
        let mut preset = FxPreset::Classic;
        for expected in [
            FxPreset::Blaze,
            FxPreset::Smear,
            FxPreset::Ripple,
            FxPreset::Fireworks,
            FxPreset::Classic,
        ] {
            preset = preset.cycle_next();
            assert_eq!(preset, expected);
        }
    }

    #[test]
    fn missing_preset_uses_classic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut text = toml::to_string_pretty(&UserConfig::default()).unwrap();
        text = text.replace("preset = \"classic\"\n", "");
        fs::write(&path, text).unwrap();

        let loaded = load(&path).unwrap();

        assert_eq!(loaded.fx.preset, FxPreset::Classic);
    }

    #[test]
    fn missing_syntax_highlight_uses_enabled_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut text = toml::to_string_pretty(&UserConfig::default()).unwrap();
        text = text.replace("syntax_highlight = true\n", "");
        fs::write(&path, text).unwrap();

        let loaded = load(&path).unwrap();

        assert!(loaded.typing.syntax_highlight);
    }

    #[test]
    fn legacy_presets_map_to_cursor_presets() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for (legacy, expected) in [
            ("flare", FxPreset::Blaze),
            ("ember", FxPreset::Blaze),
            ("comet", FxPreset::Smear),
            ("matrix", FxPreset::Smear),
            ("pulse", FxPreset::Ripple),
        ] {
            let text = toml::to_string_pretty(&UserConfig::default())
                .unwrap()
                .replace("preset = \"classic\"", &format!("preset = \"{legacy}\""));
            fs::write(&path, text).unwrap();

            assert_eq!(load(&path).unwrap().fx.preset, expected);
        }
    }

    #[test]
    fn rejects_invalid_tab_width() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        fs::write(
            &path,
            r#"
[content]
include_imports = false
include_doc_comments = true
include_comments = false
include_tests = false
include_configs = false
tab_width = 99

[typing]
auto_indent = true
auto_close_brackets = false
allow_backspace = true
show_live_speed = false

[progress]
mode = "manual"
dependency_direction = "top_down"
keep_done_on_refresh = true
hide_skipped = true

[fx]
enabled = true
intensity = "normal"
"#,
        )
        .unwrap();
        let err = load(&path).unwrap_err();
        assert!(err.to_string().contains("tab_width"));
    }

    #[test]
    fn rejects_unknown_intensity() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        let mut text = toml::to_string_pretty(&UserConfig::default()).unwrap();
        text = text.replace("intensity = \"normal\"", "intensity = \"ludicrous\"");
        fs::write(&path, text).unwrap();
        let err = load(&path).unwrap_err();
        assert!(err.to_string().contains("invalid config"));
    }

    #[test]
    fn legacy_dependency_mode_maps_to_flow() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let text = toml::to_string_pretty(&UserConfig::default())
            .unwrap()
            .replace("mode = \"manual\"", "mode = \"dependency\"");
        fs::write(&path, text).unwrap();
        assert_eq!(load(&path).unwrap().progress.mode, ProgressMode::Flow);
    }

    #[test]
    fn rejects_auto_close_brackets_true() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        let mut cfg = UserConfig::default();
        cfg.typing.auto_close_brackets = true;
        // Bypass save() validation by writing raw toml.
        let text = toml::to_string_pretty(&cfg).unwrap();
        fs::write(&path, text).unwrap();
        let err = load(&path).unwrap_err();
        assert!(err.to_string().contains("auto_close_brackets"));
    }
}
