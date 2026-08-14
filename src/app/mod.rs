//! Application state machine: places (Home / Tree / Typing / Result) plus overlays.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::Color;

use crate::config::{save as save_user_config, UserConfig};
use crate::config::{DependencyDirection, ProgressMode};
use crate::domain::content::{
    ChunkCompletion, FileStatus, ManualOverride, RepoProgress, ResolvedRepository, SessionSummary,
    TypingCheckpoint, TypingMetrics,
};
use crate::domain::dependency::{order_files, uses_dependency_mode};
use crate::domain::typing::{SessionState, TypingCommand, TypingEngine};
use crate::scan::extract::ExtractOptions;
use crate::scan::walk::{scan_repository, single_file_scan, WalkOptions};
use crate::source::resolve_source;
use crate::store::SqliteStore;
use crate::ui::fx::FxState;
use crate::ui::help::{draw_help, HelpContext};
use crate::ui::highlight::highlight_chars;
use crate::ui::home::{draw_home, draw_search_modal, HomeView};
use crate::ui::pause::draw_pause;
use crate::ui::result::{draw_result, ResultView};
use crate::ui::search::SearchState;
use crate::ui::settings::{draw_settings, SettingKind, SettingsView};
use crate::ui::splash::{draw_splash, looping_logo_elapsed, SPLASH_TOTAL_MS};
use crate::ui::stats::{draw_stats, StatsView};
use crate::ui::terminal::TerminalGuard;
use crate::ui::tree::{draw_tree, TreeView};
use crate::ui::typing::draw_typing;
use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Home,
    Tree,
    Typing,
    Result,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overlay {
    Search,
    Settings,
    Stats,
    Pause,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub cache_dir: PathBuf,
    pub db_path: PathBuf,
    pub refresh: bool,
    /// CLI `--no-fx` session override.
    pub no_fx: bool,
    pub user: UserConfig,
    pub config_path: PathBuf,
}

impl AppConfig {
    pub fn fx_enabled(&self) -> bool {
        !self.no_fx && self.user.fx_active()
    }
}

/// Active repository session (present whenever Tree / Typing / Result / Splash is shown).
struct RepoSession {
    repo: ResolvedRepository,
    repo_id: i64,
    progress: RepoProgress,
    tree: TreeView,
    engine: Option<TypingEngine>,
    typing_path: String,
    typing_chunk_label: String,
    typing_syntax_colors: Option<Vec<Color>>,
    typing_chunk_id: i64,
    session_started_at: String,
    typing_auto_indent: bool,
    typing_checkpoint_last_saved_ms: u64,
    result: Option<ResultView>,
    single_file: Option<String>,
    import_edges: Vec<(String, String)>,
    dependency_direction: DependencyDirection,
    dependency_mode: bool,
    entry_override: Option<String>,
    ordered_paths: Option<Vec<String>>,
}

struct DependencySettings {
    import_edges: Vec<(String, String)>,
    mode: ProgressMode,
    direction: DependencyDirection,
}

pub struct App {
    cfg: AppConfig,
    store: SqliteStore,
    place: Place,
    overlay: Option<Overlay>,
    help: bool,
    showing_splash: bool,
    home: HomeView,
    search: SearchState,
    stats: Option<StatsView>,
    settings: SettingsView,
    session: Option<RepoSession>,
    fx: FxState,
    splash_started: Option<Instant>,
}

impl App {
    /// Open a repository and start on Tree (optionally Splash). Home is skipped.
    pub fn open(input: &str, cfg: &AppConfig) -> crate::Result<Self> {
        let mut store = SqliteStore::open(&cfg.db_path)?;
        let session = load_session(input, cfg, &mut store)?;
        let home = load_home_view(&store)?;
        Ok(Self {
            cfg: cfg.clone(),
            store,
            place: Place::Tree,
            overlay: None,
            help: false,
            showing_splash: false,
            home,
            search: SearchState::default(),
            stats: None,
            settings: SettingsView::new(),
            session: Some(session),
            fx: FxState::with_config(cfg.user.fx.intensity, cfg.user.fx.preset),
            splash_started: None,
        })
    }

    /// Start on the Home screen with no repository loaded.
    pub fn home(cfg: &AppConfig) -> crate::Result<Self> {
        let store = SqliteStore::open(&cfg.db_path)?;
        let home = load_home_view(&store)?;
        Ok(Self {
            cfg: cfg.clone(),
            store,
            place: Place::Home,
            overlay: None,
            help: false,
            showing_splash: false,
            home,
            search: SearchState::default(),
            stats: None,
            settings: SettingsView::new(),
            session: None,
            fx: FxState::with_config(cfg.user.fx.intensity, cfg.user.fx.preset),
            splash_started: None,
        })
    }

    pub fn progress(&self) -> &RepoProgress {
        &self
            .session
            .as_ref()
            .expect("progress requires an open repository")
            .progress
    }

    pub fn run(&mut self) -> crate::Result<()> {
        let mut guard = TerminalGuard::enter()?;
        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(16);

        // Arg-launch path: optional splash before Tree.
        if self.session.is_some() && self.place == Place::Tree && self.cfg.fx_enabled() {
            self.showing_splash = true;
            self.splash_started = Some(Instant::now());
        }
        // Home path: always play the logo reveal/glow on the title screen.
        if self.place == Place::Home {
            self.splash_started = Some(Instant::now());
        }

        loop {
            let now_ms = now_millis();
            if self.place == Place::Tree {
                if let Some(s) = &mut self.session {
                    s.tree.visible_rows = 12;
                }
            }
            let typing_snap = if self.place == Place::Typing {
                self.session
                    .as_ref()
                    .and_then(|s| s.engine.as_ref().map(|e| e.snapshot()))
            } else {
                None
            };
            if self.cfg.fx_enabled() {
                if let Some(snap) = &typing_snap {
                    self.fx.observe(snap, now_ms);
                }
            }
            let splash_elapsed = self
                .splash_started
                .map(|t| t.elapsed().as_millis() as u64)
                .unwrap_or(0);
            // Home logo keeps sweeping after the initial reveal; `--no-fx` shows
            // the fully revealed logo without animation.
            let home_logo_elapsed = if self.cfg.fx_enabled() {
                looping_logo_elapsed(splash_elapsed)
            } else {
                SPLASH_TOTAL_MS
            };
            {
                let term = guard.terminal();
                term.draw(|frame| {
                    let area = frame.area();
                    if self.showing_splash {
                        if let Some(s) = &self.session {
                            draw_splash(frame, area, splash_elapsed, &s.repo.display_name);
                        }
                    } else {
                        match self.place {
                            Place::Home => {
                                draw_home(frame, area, &self.home, home_logo_elapsed);
                            }
                            Place::Tree => {
                                if let Some(s) = &self.session {
                                    draw_tree(frame, area, &s.tree);
                                }
                            }
                            Place::Typing => {
                                if let (Some(s), Some(snap)) = (&self.session, &typing_snap) {
                                    let location =
                                        format!("{} › {}", s.repo.display_name, s.typing_path);
                                    draw_typing(
                                        frame,
                                        area,
                                        &location,
                                        &s.typing_chunk_label,
                                        snap,
                                        s.typing_syntax_colors.as_deref(),
                                        now_ms,
                                        self.cfg.fx_enabled().then_some(&self.fx),
                                        self.cfg.user.typing.show_live_speed,
                                    );
                                }
                            }
                            Place::Result => {
                                if let Some(view) =
                                    self.session.as_ref().and_then(|s| s.result.as_ref())
                                {
                                    draw_result(frame, area, view);
                                }
                            }
                        }
                        match self.overlay {
                            Some(Overlay::Search) => {
                                draw_search_modal(frame, area, &self.search);
                            }
                            Some(Overlay::Settings) => {
                                draw_settings(frame, area, &self.settings, &self.cfg.user);
                            }
                            Some(Overlay::Stats) => {
                                if let Some(stats) = &self.stats {
                                    draw_stats(frame, area, stats);
                                }
                            }
                            Some(Overlay::Pause) => draw_pause(frame, area),
                            None => {}
                        }
                        if self.help {
                            draw_help(frame, area, self.help_context());
                        }
                    }
                })
                .map_err(|e| Error::Terminal(e.to_string()))?;
            }

            let timeout = tick_rate.saturating_sub(last_tick.elapsed());
            if event::poll(timeout).map_err(|e| Error::Terminal(e.to_string()))? {
                if let Event::Key(key) =
                    event::read().map_err(|e| Error::Terminal(e.to_string()))?
                {
                    if (key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat)
                        && self.handle_key(key)?
                    {
                        break;
                    }
                }
            }

            if last_tick.elapsed() >= tick_rate {
                if self.showing_splash && splash_elapsed >= SPLASH_TOTAL_MS {
                    self.showing_splash = false;
                }
                let finished = self.session.as_mut().and_then(|session| {
                    let engine = session.engine.as_mut()?;
                    let now_ms = now_millis();
                    engine.apply(TypingCommand::Tick { now_ms });
                    let state = engine.snapshot().state;
                    if state == SessionState::Completed {
                        Some(true)
                    } else {
                        None
                    }
                });
                if let Some(completed) = finished {
                    self.finish_typing(completed)?;
                } else if self.place == Place::Typing {
                    self.autosave_checkpoint(false)?;
                }
                last_tick = Instant::now();
            }
        }

        guard.restore()?;
        if let Some(s) = &self.session {
            let _ = self.store.touch_repository(s.repo_id);
        }
        Ok(())
    }

    /// Returns true when the app should quit.
    fn handle_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if self.place == Place::Typing {
                self.interrupt_typing()?;
            }
            return Ok(true);
        }

        if self.showing_splash {
            self.showing_splash = false;
            return Ok(false);
        }

        if self.help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?')
            ) {
                self.help = false;
            }
            return Ok(false);
        }

        if self.overlay == Some(Overlay::Pause) {
            return self.handle_pause_key(key);
        }

        if self.place == Place::Typing {
            return self.handle_typing_key(key);
        }

        if key.code == KeyCode::Char('?') {
            self.help = true;
            return Ok(false);
        }

        match self.overlay {
            Some(Overlay::Search) => return self.handle_search_key(key),
            Some(Overlay::Settings) => return self.handle_settings_key(key),
            Some(Overlay::Stats) => return self.handle_stats_key(key),
            Some(Overlay::Pause) | None => {}
        }

        if key.code == KeyCode::Char(',')
            || (self.place == Place::Home && key.code == KeyCode::Char('c'))
        {
            self.open_settings();
            return Ok(false);
        }
        if key.code == KeyCode::Char('S') {
            self.open_stats()?;
            return Ok(false);
        }
        if key.code == KeyCode::Char('/') && self.place == Place::Home
            || (self.place == Place::Home && key.code == KeyCode::Char('s'))
        {
            self.open_search();
            return Ok(false);
        }

        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            return self.back_or_quit(key.code == KeyCode::Char('q'));
        }

        match self.place {
            Place::Home => self.handle_home_key(key),
            Place::Tree => self.handle_tree_key(key),
            Place::Typing => self.handle_typing_key(key),
            Place::Result => self.handle_result_key(key),
        }
    }

    fn help_context(&self) -> HelpContext {
        if self.help {
            if let Some(overlay) = self.overlay {
                return match overlay {
                    Overlay::Search => HelpContext::Search,
                    Overlay::Settings => HelpContext::Settings,
                    Overlay::Stats => HelpContext::Stats,
                    Overlay::Pause => HelpContext::Pause,
                };
            }
        }
        match self.place {
            Place::Home => HelpContext::Home,
            Place::Tree => HelpContext::Tree,
            Place::Typing => HelpContext::Typing,
            Place::Result => HelpContext::Result,
        }
    }

    fn open_search(&mut self) {
        self.search = SearchState::default();
        self.search.refresh(&self.home.recent, &self.cfg.cache_dir);
        self.overlay = Some(Overlay::Search);
    }

    fn open_settings(&mut self) {
        self.settings = SettingsView::new();
        self.overlay = Some(Overlay::Settings);
    }

    fn open_stats(&mut self) -> crate::Result<()> {
        self.reload_home_data()?;
        self.stats = Some(StatsView::new(
            self.home.summary.clone(),
            self.home.recent.clone(),
        ));
        self.overlay = Some(Overlay::Stats);
        Ok(())
    }

    fn close_overlay(&mut self) -> crate::Result<()> {
        if self.overlay == Some(Overlay::Settings) {
            self.apply_live_settings()?;
        }
        self.overlay = None;
        Ok(())
    }

    fn apply_live_settings(&mut self) -> crate::Result<()> {
        self.fx.set_intensity(self.cfg.user.fx.intensity);
        self.fx.set_preset(self.cfg.user.fx.preset);
        if let Some(s) = &mut self.session {
            s.tree.hide_skipped = self.cfg.user.progress.hide_skipped;
            s.tree.refresh_rows(&s.progress);
        }
        Ok(())
    }

    fn back_or_quit(&mut self, from_q: bool) -> crate::Result<bool> {
        match self.place {
            Place::Home => Ok(from_q || false),
            Place::Tree => {
                self.return_to_home()?;
                Ok(false)
            }
            Place::Result => {
                self.return_to_tree();
                Ok(false)
            }
            Place::Typing => Ok(false),
        }
    }

    fn return_to_tree(&mut self) {
        if let Some(s) = &mut self.session {
            s.result = None;
            s.tree.refresh_rows(&s.progress);
        }
        self.place = Place::Tree;
        self.overlay = None;
        self.help = false;
    }

    fn handle_home_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.home.move_by(1);
                Ok(false)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.home.move_by(-1);
                Ok(false)
            }
            KeyCode::Char('g') => {
                self.home.select_first();
                Ok(false)
            }
            KeyCode::Char('G') => {
                self.home.select_last();
                Ok(false)
            }
            KeyCode::Enter => {
                if let Some(input) = self.home.selected_input().map(str::to_string) {
                    self.open_from_home(&input)?;
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.close_overlay()?;
                Ok(false)
            }
            KeyCode::Enter => {
                if let Some(input) = self.search.confirm_input() {
                    self.open_from_home(&input)?;
                }
                Ok(false)
            }
            KeyCode::Char('n') | KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.move_by(1);
                Ok(false)
            }
            KeyCode::Char('p') | KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.move_by(-1);
                Ok(false)
            }
            KeyCode::Down => {
                self.search.move_by(1);
                Ok(false)
            }
            KeyCode::Up => {
                self.search.move_by(-1);
                Ok(false)
            }
            KeyCode::Backspace => {
                self.search.query.pop();
                self.search.refresh(&self.home.recent, &self.cfg.cache_dir);
                Ok(false)
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.query.push(c);
                self.search.refresh(&self.home.recent, &self.cfg.cache_dir);
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_stats_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('S') => {
                self.close_overlay()?;
                Ok(false)
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(stats) = &mut self.stats {
                    stats.move_by(1);
                }
                Ok(false)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(stats) = &mut self.stats {
                    stats.move_by(-1);
                }
                Ok(false)
            }
            KeyCode::Char('g') => {
                if let Some(stats) = &mut self.stats {
                    stats.select_first();
                }
                Ok(false)
            }
            KeyCode::Char('G') => {
                if let Some(stats) = &mut self.stats {
                    stats.select_last();
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_settings_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char(',') => {
                self.close_overlay()?;
                Ok(false)
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.settings.move_by(1);
                Ok(false)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.settings.move_by(-1);
                Ok(false)
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if self.settings.activate(&mut self.cfg.user, true) {
                    self.persist_user_config()?;
                }
                Ok(false)
            }
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Char('+') => {
                let kind = self.settings.selected_kind();
                if matches!(
                    kind,
                    SettingKind::TabWidth
                        | SettingKind::FxIntensity
                        | SettingKind::FxPreset
                        | SettingKind::ProgressMode
                        | SettingKind::DependencyDirection
                        | SettingKind::Bool
                ) && self.settings.activate(&mut self.cfg.user, true)
                {
                    self.persist_user_config()?;
                }
                Ok(false)
            }
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Char('-') => {
                let kind = self.settings.selected_kind();
                if matches!(
                    kind,
                    SettingKind::TabWidth
                        | SettingKind::FxIntensity
                        | SettingKind::FxPreset
                        | SettingKind::ProgressMode
                        | SettingKind::DependencyDirection
                ) {
                    if self.settings.activate(&mut self.cfg.user, false) {
                        self.persist_user_config()?;
                    }
                } else if kind == SettingKind::Bool
                    && self.settings.activate(&mut self.cfg.user, false)
                {
                    self.persist_user_config()?;
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn persist_user_config(&mut self) -> crate::Result<()> {
        save_user_config(&self.cfg.config_path, &self.cfg.user)?;
        self.fx.set_intensity(self.cfg.user.fx.intensity);
        self.fx.set_preset(self.cfg.user.fx.preset);
        Ok(())
    }

    fn handle_tree_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        if self.session.as_ref().is_some_and(|s| s.tree.filter_editing) {
            return self.handle_tree_filter_key(key);
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(s) = &mut self.session {
                    s.tree.move_by(1);
                }
                Ok(false)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(s) = &mut self.session {
                    s.tree.move_by(-1);
                }
                Ok(false)
            }
            KeyCode::Char('g') => {
                if let Some(s) = &mut self.session {
                    s.tree.select_first();
                }
                Ok(false)
            }
            KeyCode::Char('G') => {
                if let Some(s) = &mut self.session {
                    s.tree.select_last();
                }
                Ok(false)
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(s) = &mut self.session {
                    s.tree.page_by(1);
                }
                Ok(false)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(s) = &mut self.session {
                    s.tree.page_by(-1);
                }
                Ok(false)
            }
            KeyCode::Char(' ') | KeyCode::Char('l') | KeyCode::Right => {
                if let Some(s) = &mut self.session {
                    let progress = s.progress.clone();
                    s.tree.expand_dir(&progress);
                }
                Ok(false)
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if let Some(s) = &mut self.session {
                    let progress = s.progress.clone();
                    s.tree.collapse_or_parent(&progress);
                }
                Ok(false)
            }
            KeyCode::Tab => {
                if let Some(s) = &mut self.session {
                    s.tree.jump_recommend();
                }
                Ok(false)
            }
            KeyCode::Char('n') => {
                if let Some(s) = &mut self.session {
                    if !s.tree.filter.is_empty() {
                        s.tree.next_match(false);
                    }
                }
                Ok(false)
            }
            KeyCode::Char('N') => {
                if let Some(s) = &mut self.session {
                    if !s.tree.filter.is_empty() {
                        s.tree.next_match(true);
                    }
                }
                Ok(false)
            }
            KeyCode::Char('/') => {
                if let Some(s) = &mut self.session {
                    s.tree.begin_filter();
                }
                Ok(false)
            }
            KeyCode::Char('x') => {
                self.toggle_tree_skip()?;
                Ok(false)
            }
            KeyCode::Char('X') => {
                self.clear_tree_skip()?;
                Ok(false)
            }
            KeyCode::Char('e') => {
                if let Some(s) = &mut self.session {
                    if s.dependency_mode {
                        if let Some(path) = s.tree.selected_file_path() {
                            let is_typeable = s.progress.files.iter().any(|file| {
                                file.relative_path == path
                                    && file.derive_status() != FileStatus::Skipped
                            });
                            if is_typeable {
                                s.entry_override = Some(path);
                                s.recompute_dependency_order();
                            }
                        }
                    }
                }
                Ok(false)
            }
            KeyCode::Enter => {
                if let Some(s) = &mut self.session {
                    if s.tree.selected_dir_path().is_some() {
                        let progress = s.progress.clone();
                        s.tree.toggle_collapse(&progress);
                        return Ok(false);
                    }
                }
                let path = self
                    .session
                    .as_ref()
                    .and_then(|s| s.tree.selected_file_path());
                if let Some(path) = path {
                    self.open_file(&path)?;
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_tree_filter_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                if let Some(s) = &mut self.session {
                    let progress = s.progress.clone();
                    s.tree.clear_filter(&progress);
                }
                Ok(false)
            }
            KeyCode::Char('n') => {
                if let Some(s) = &mut self.session {
                    s.tree.next_match(false);
                }
                Ok(false)
            }
            KeyCode::Char('N') => {
                if let Some(s) = &mut self.session {
                    s.tree.next_match(true);
                }
                Ok(false)
            }
            KeyCode::Backspace => {
                if let Some(s) = &mut self.session {
                    let progress = s.progress.clone();
                    s.tree.pop_filter(&progress);
                }
                Ok(false)
            }
            KeyCode::Enter => {
                if let Some(s) = &mut self.session {
                    s.tree.filter_editing = false;
                }
                Ok(false)
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(s) = &mut self.session {
                    let progress = s.progress.clone();
                    s.tree.push_filter(c, &progress);
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_typing_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        if key.code == KeyCode::Esc {
            self.autosave_checkpoint(true)?;
            self.overlay = Some(Overlay::Pause);
            return Ok(false);
        }
        let cmd = match key.code {
            KeyCode::Enter => TypingCommand::Enter,
            KeyCode::Backspace => TypingCommand::Backspace,
            KeyCode::Tab => TypingCommand::Char('\t'),
            KeyCode::Char(c) => TypingCommand::Char(c),
            _ => return Ok(false),
        };
        let finished = self.session.as_mut().and_then(|session| {
            let engine = session.engine.as_mut()?;
            engine.apply(TypingCommand::Tick {
                now_ms: now_millis(),
            });
            engine.apply(cmd);
            let state = engine.snapshot().state;
            if state == SessionState::Completed {
                Some(true)
            } else {
                None
            }
        });
        if let Some(completed) = finished {
            self.finish_typing(completed)?;
        }
        Ok(false)
    }

    fn handle_pause_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        if key.code == KeyCode::Char('?') {
            self.help = true;
            return Ok(false);
        }
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                self.overlay = None;
                Ok(false)
            }
            KeyCode::Char('r') => {
                self.overlay = None;
                let path = self
                    .session
                    .as_ref()
                    .map(|s| s.typing_path.clone())
                    .unwrap_or_default();
                if !path.is_empty() {
                    self.reset_typing_session()?;
                    self.start_file(&path, 0)?;
                }
                Ok(false)
            }
            KeyCode::Char('t') => {
                self.overlay = None;
                self.interrupt_typing()?;
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_result_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Enter => {
                let next = self
                    .session
                    .as_ref()
                    .and_then(|s| s.tree.recommend.clone())
                    .or_else(|| {
                        self.session
                            .as_ref()
                            .and_then(|s| s.progress.recommend_path().map(str::to_string))
                    });
                if let Some(path) = next {
                    self.open_file(&path)?;
                } else {
                    if let Some(s) = &mut self.session {
                        s.tree.repo_complete = s.progress.is_repo_complete();
                    }
                    self.return_to_tree();
                }
                Ok(false)
            }
            KeyCode::Char('r') => {
                let path = self
                    .session
                    .as_ref()
                    .and_then(|s| s.result.as_ref().map(|r| r.path.clone()));
                if let Some(path) = path {
                    self.open_file(&path)?;
                }
                Ok(false)
            }
            KeyCode::Char('t') => {
                self.return_to_tree();
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn interrupt_typing(&mut self) -> crate::Result<()> {
        self.autosave_checkpoint(true)?;
        if let Some(session) = &mut self.session {
            session.engine = None;
            session.result = None;
            let progress = session.progress.clone();
            session.tree.refresh_rows(&progress);
        }
        self.place = Place::Tree;
        self.overlay = None;
        Ok(())
    }

    fn toggle_tree_skip(&mut self) -> crate::Result<()> {
        let paths = self.selected_override_paths();
        let updates: Vec<(String, Option<ManualOverride>)> = {
            let Some(session) = &self.session else {
                return Ok(());
            };
            paths
                .into_iter()
                .filter_map(|path| {
                    session
                        .progress
                        .files
                        .iter()
                        .find(|f| f.relative_path == path)
                        .and_then(|f| {
                            f.toggle_override_target()
                                .map(|target| (path, Some(target)))
                        })
                })
                .collect()
        };
        self.apply_overrides(updates)
    }

    fn clear_tree_skip(&mut self) -> crate::Result<()> {
        let updates = self
            .selected_override_paths()
            .into_iter()
            .map(|path| (path, None))
            .collect();
        self.apply_overrides(updates)
    }

    fn selected_override_paths(&self) -> Vec<String> {
        let Some(session) = &self.session else {
            return Vec::new();
        };
        if let Some(file) = session.tree.selected_file_path() {
            return vec![file];
        }
        if let Some(dir) = session.tree.selected_dir_path() {
            let prefix = format!("{dir}/");
            return session
                .progress
                .files
                .iter()
                .filter(|f| f.relative_path == dir || f.relative_path.starts_with(&prefix))
                .map(|f| f.relative_path.clone())
                .collect();
        }
        Vec::new()
    }

    fn apply_overrides(
        &mut self,
        updates: Vec<(String, Option<ManualOverride>)>,
    ) -> crate::Result<()> {
        let Some(session) = &mut self.session else {
            return Ok(());
        };
        let repo_id = session.repo_id;
        for (path, value) in &updates {
            self.store.set_manual_override(repo_id, path, *value)?;
            if let Some(file) = session
                .progress
                .files
                .iter_mut()
                .find(|f| f.relative_path == *path)
            {
                file.manual_override = *value;
                file.status = file.derive_status();
            }
        }
        if session.dependency_mode {
            session.recompute_dependency_order();
        } else {
            session.tree.refresh_rows(&session.progress);
        }
        Ok(())
    }

    fn open_from_home(&mut self, input: &str) -> crate::Result<()> {
        match load_session(input, &self.cfg, &mut self.store) {
            Ok(session) => {
                self.session = Some(session);
                self.home.error = None;
                self.place = Place::Tree;
                self.overlay = None;
                self.help = false;
                self.showing_splash = false;
                self.splash_started = None;
                Ok(())
            }
            Err(err) => {
                self.home.error = Some(err.to_string());
                self.place = Place::Home;
                self.overlay = None;
                Ok(())
            }
        }
    }

    fn return_to_home(&mut self) -> crate::Result<()> {
        if let Some(s) = &self.session {
            let _ = self.store.touch_repository(s.repo_id);
        }
        self.session = None;
        self.stats = None;
        self.search = SearchState::default();
        self.overlay = None;
        self.help = false;
        self.reload_home_data()?;
        self.place = Place::Home;
        self.splash_started = Some(Instant::now());
        Ok(())
    }

    fn reload_home_data(&mut self) -> crate::Result<()> {
        self.home = load_home_view(&self.store)?;
        Ok(())
    }

    fn open_file(&mut self, path: &str) -> crate::Result<()> {
        let Some(session) = &self.session else {
            return Ok(());
        };
        let file = session
            .progress
            .files
            .iter()
            .find(|f| f.relative_path == path)
            .ok_or_else(|| Error::Message(format!("file not found: {path}")))?;
        if file.derive_status() == FileStatus::Skipped {
            return Ok(());
        }
        if file.chunks.is_empty() {
            return Ok(());
        }
        self.start_file(path, 0)?;
        Ok(())
    }

    fn start_file(&mut self, path: &str, idx: usize) -> crate::Result<()> {
        let allow_backspace = self.cfg.user.typing.allow_backspace;
        let auto_indent = self.cfg.user.typing.auto_indent;
        let tab_width = self.cfg.user.content.tab_width;
        let fx_intensity = self.cfg.user.fx.intensity;
        let fx_preset = self.cfg.user.fx.preset;

        let Some(session) = &mut self.session else {
            return Err(Error::Message("no repository open".into()));
        };
        let file = session
            .progress
            .files
            .iter()
            .find(|f| f.relative_path == path)
            .ok_or_else(|| Error::Message(format!("file not found: {path}")))?;
        let cp = file
            .chunks
            .get(idx)
            .ok_or_else(|| Error::Message("file body missing".into()))?;
        let chunk_id = cp
            .id
            .ok_or_else(|| Error::Message("file body missing database id".into()))?;
        let normalized = cp.chunk.normalized.clone();
        let checkpoint = cp.checkpoint.clone();
        let label = format!("lines {}–{}", cp.chunk.start_line, cp.chunk.end_line);
        let syntax_colors = self
            .cfg
            .user
            .typing
            .syntax_highlight
            .then(|| highlight_chars(path, &normalized));

        let started_ms = now_millis();
        let (engine, session_started_at, session_auto_indent) = if let Some(checkpoint) = checkpoint
        {
            let engine = TypingEngine::from_checkpoint(
                &normalized,
                started_ms,
                &checkpoint,
                allow_backspace,
                tab_width,
            )
            .map_err(|error| {
                Error::Message(format!("invalid typing checkpoint for {path}: {error:?}"))
            })?;
            (engine, checkpoint.started_at, checkpoint.auto_indent)
        } else {
            (
                TypingEngine::new(
                    &normalized,
                    started_ms,
                    allow_backspace,
                    auto_indent,
                    tab_width,
                ),
                Utc::now().to_rfc3339(),
                auto_indent,
            )
        };
        session.engine = Some(engine);
        self.fx = FxState::with_config(fx_intensity, fx_preset);
        session.typing_path = path.to_string();
        session.typing_chunk_label = label;
        session.typing_syntax_colors = syntax_colors;
        session.typing_chunk_id = chunk_id;
        session.session_started_at = session_started_at;
        session.typing_auto_indent = session_auto_indent;
        session.typing_checkpoint_last_saved_ms = 0;
        session.result = None;
        session.tree.repo_complete = false;
        self.place = Place::Typing;
        self.overlay = None;

        let already_done = session
            .engine
            .as_ref()
            .is_some_and(|e| e.snapshot().state == SessionState::Completed);
        let _ = session.single_file;
        if already_done {
            self.finish_typing(true)?;
        }
        Ok(())
    }

    /// Persist the current engine state, at most once per second unless forced.
    fn autosave_checkpoint(&mut self, force: bool) -> crate::Result<()> {
        let now_ms = now_millis();
        let should_save = self
            .session
            .as_ref()
            .map(|session| {
                force || now_ms.saturating_sub(session.typing_checkpoint_last_saved_ms) >= 1_000
            })
            .unwrap_or(false);
        if !should_save {
            return Ok(());
        }

        let checkpoint = {
            let Some(session) = &mut self.session else {
                return Ok(());
            };
            let Some(engine) = &mut session.engine else {
                return Ok(());
            };
            engine.apply(TypingCommand::Tick { now_ms });
            let snapshot = engine.snapshot();
            TypingCheckpoint {
                chunk_id: session.typing_chunk_id,
                cursor: snapshot.cursor,
                keystrokes: snapshot.keystrokes,
                misses: snapshot.misses,
                elapsed_ms: snapshot.elapsed_ms,
                started_at: session.session_started_at.clone(),
                auto_indent: session.typing_auto_indent,
            }
        };

        self.store.save_checkpoint(&checkpoint)?;
        if let Some(session) = &mut self.session {
            session.typing_checkpoint_last_saved_ms = now_ms;
            if let Some(file) = session
                .progress
                .files
                .iter_mut()
                .find(|file| file.relative_path == session.typing_path)
            {
                if let Some(chunk) = file
                    .chunks
                    .iter_mut()
                    .find(|chunk| chunk.id == Some(checkpoint.chunk_id))
                {
                    chunk.checkpoint = Some(checkpoint);
                }
            }
        }
        Ok(())
    }

    /// Finalize an abandoned session before starting it over from the beginning.
    fn reset_typing_session(&mut self) -> crate::Result<()> {
        self.autosave_checkpoint(true)?;
        let (summary, chunk_id) = {
            let Some(session) = &mut self.session else {
                return Ok(());
            };
            let Some(engine) = session.engine.take() else {
                return Ok(());
            };
            let snapshot = engine.snapshot();
            (
                SessionSummary {
                    chunk_id: session.typing_chunk_id,
                    started_at: session.session_started_at.clone(),
                    ended_at: Utc::now().to_rfc3339(),
                    completed: false,
                    keystrokes: snapshot.keystrokes,
                    misses: snapshot.misses,
                    elapsed_ms: snapshot.elapsed_ms,
                },
                session.typing_chunk_id,
            )
        };
        self.store.record_session(&summary)?;
        if let Some(session) = &mut self.session {
            if let Some(file) = session
                .progress
                .files
                .iter_mut()
                .find(|file| file.relative_path == session.typing_path)
            {
                if let Some(chunk) = file
                    .chunks
                    .iter_mut()
                    .find(|chunk| chunk.id == Some(chunk_id))
                {
                    chunk.checkpoint = None;
                }
            }
        }
        Ok(())
    }

    fn finish_typing(&mut self, completed: bool) -> crate::Result<()> {
        let Some(session) = &mut self.session else {
            return Ok(());
        };
        let Some(engine) = session.engine.take() else {
            return Ok(());
        };
        let snap = engine.snapshot();
        let metrics = TypingMetrics::from_counts(snap.keystrokes, snap.misses, snap.elapsed_ms);
        let ended = Utc::now().to_rfc3339();
        let summary = SessionSummary {
            chunk_id: session.typing_chunk_id,
            started_at: session.session_started_at.clone(),
            ended_at: ended,
            completed,
            keystrokes: snap.keystrokes,
            misses: snap.misses,
            elapsed_ms: snap.elapsed_ms,
        };
        let typing_path = session.typing_path.clone();
        let typing_chunk_id = session.typing_chunk_id;

        if completed {
            if let Some(file) = session
                .progress
                .files
                .iter_mut()
                .find(|f| f.relative_path == typing_path)
            {
                if let Some(c) = file
                    .chunks
                    .iter_mut()
                    .find(|c| c.id == Some(typing_chunk_id))
                {
                    c.completion = ChunkCompletion::Complete;
                    c.checkpoint = None;
                }
                file.status = file.derive_status();
            }
        }

        let file_done = session
            .progress
            .files
            .iter()
            .find(|f| f.relative_path == typing_path)
            .map(|f| f.derive_status() == FileStatus::Done)
            .unwrap_or(false);

        self.store.record_session(&summary)?;
        session.tree.refresh_rows(&session.progress);
        if completed {
            session.result = Some(ResultView {
                repo: session.repo.display_name.clone(),
                path: typing_path,
                completed,
                metrics,
                file_done,
            });
            self.place = Place::Result;
        } else {
            session.result = None;
            self.place = Place::Tree;
        }
        Ok(())
    }
}

fn load_home_view(store: &SqliteStore) -> crate::Result<HomeView> {
    let recent = store.list_recent_repos(20)?;
    let summary = store.global_summary()?;
    Ok(HomeView::new(recent, summary))
}

fn load_session(
    input: &str,
    cfg: &AppConfig,
    store: &mut SqliteStore,
) -> crate::Result<RepoSession> {
    let resolved = resolve_source(input, &cfg.cache_dir, cfg.refresh)?;
    let walk = walk_options(&cfg.user);

    let (progress_repo, scan, single_file) = if resolved.root.is_dir() {
        if resolved.identity.starts_with("local:") {
            let local_path = resolved.identity.trim_start_matches("local:");
            let p = PathBuf::from(local_path);
            if p.is_file() {
                let (parent, scan) = single_file_scan(&p, walk)?;
                let name = p
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let mut resolved_root = resolved.clone();
                resolved_root.root = parent;
                let import_edges = scan.import_edges.clone();
                let (repo_id, progress) = store.sync_scan(
                    &resolved_root,
                    &scan,
                    cfg.user.progress.keep_done_on_refresh,
                )?;
                if !progress
                    .files
                    .iter()
                    .any(|f| f.derive_status() != FileStatus::Skipped)
                {
                    return Err(Error::NoChunks);
                }
                return Ok(RepoSession::from_parts(
                    resolved_root,
                    repo_id,
                    progress,
                    Some(name),
                    DependencySettings {
                        import_edges,
                        mode: cfg.user.progress.mode,
                        direction: cfg.user.progress.dependency_direction,
                    },
                    cfg.user.progress.hide_skipped,
                ));
            }
        }
        let scan = scan_repository(&resolved.root, walk)?;
        (resolved, scan, None)
    } else {
        return Err(Error::InvalidPath(resolved.root));
    };

    if !scan.has_typeable_content() {
        return Err(Error::NoChunks);
    }
    let (repo_id, progress) = store.sync_scan(
        &progress_repo,
        &scan,
        cfg.user.progress.keep_done_on_refresh,
    )?;
    let import_edges = scan.import_edges.clone();
    Ok(RepoSession::from_parts(
        progress_repo,
        repo_id,
        progress,
        single_file,
        DependencySettings {
            import_edges,
            mode: cfg.user.progress.mode,
            direction: cfg.user.progress.dependency_direction,
        },
        cfg.user.progress.hide_skipped,
    ))
}

fn walk_options(user: &UserConfig) -> WalkOptions {
    WalkOptions {
        max_line_cols: 200,
        max_file_lines: 5_000,
        include_tests: user.content.include_tests,
        include_configs: user.content.include_configs,
        extract: ExtractOptions {
            tab_width: user.content.tab_width.max(1),
            include_imports: user.content.include_imports,
            include_doc_comments: user.content.include_doc_comments,
            include_comments: user.content.include_comments,
        },
    }
}

impl RepoSession {
    fn from_parts(
        repo: ResolvedRepository,
        repo_id: i64,
        progress: RepoProgress,
        single_file: Option<String>,
        dependency: DependencySettings,
        hide_skipped: bool,
    ) -> Self {
        let dependency_mode = uses_dependency_mode(dependency.mode)
            && !dependency.import_edges.is_empty()
            && progress
                .files
                .iter()
                .filter(|file| file.derive_status() != FileStatus::Skipped)
                .count()
                > 1;
        let ordered_paths = dependency_mode.then(|| {
            dependency_order(
                &progress,
                &dependency.import_edges,
                dependency.direction,
                None,
            )
        });
        let tree = TreeView::from_progress_with_order(
            &repo.display_name,
            &progress,
            hide_skipped,
            ordered_paths.clone(),
        );
        Self {
            repo,
            repo_id,
            progress,
            tree,
            engine: None,
            typing_path: String::new(),
            typing_chunk_label: String::new(),
            typing_syntax_colors: None,
            typing_chunk_id: 0,
            session_started_at: String::new(),
            typing_auto_indent: false,
            typing_checkpoint_last_saved_ms: 0,
            result: None,
            single_file,
            import_edges: dependency.import_edges,
            dependency_direction: dependency.direction,
            dependency_mode,
            entry_override: None,
            ordered_paths,
        }
    }

    fn recompute_dependency_order(&mut self) {
        let order = if self.dependency_mode {
            Some(dependency_order(
                &self.progress,
                &self.import_edges,
                self.dependency_direction,
                self.entry_override.as_deref(),
            ))
        } else {
            None
        };
        self.ordered_paths = order.clone();
        self.tree.set_dependency_order(&self.progress, order);
    }
}

fn dependency_order(
    progress: &RepoProgress,
    edges: &[(String, String)],
    direction: DependencyDirection,
    entry_override: Option<&str>,
) -> Vec<String> {
    let mut typeable_paths: Vec<String> = progress
        .files
        .iter()
        .filter(|file| file.derive_status() != FileStatus::Skipped)
        .map(|file| file.relative_path.clone())
        .collect();
    typeable_paths.sort();
    typeable_paths.dedup();

    let entry = entry_override.or_else(|| most_imported_path(edges, &typeable_paths));
    order_files(edges, entry, direction, &typeable_paths)
}

fn most_imported_path<'a>(edges: &[(String, String)], paths: &'a [String]) -> Option<&'a str> {
    let mut best: Option<&String> = None;
    let mut best_count = 0usize;
    for path in paths {
        let count = edges.iter().filter(|(_, target)| target == path).count();
        if count > best_count {
            best = Some(path);
            best_count = count;
        }
    }
    best.map(String::as_str)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Headless helper for integration tests: open, complete first file programmatically.
pub mod headless {
    use super::*;

    pub fn open_local(path: &str, db: &Path, cache: &Path) -> crate::Result<App> {
        open_local_with_user_config(path, db, cache, UserConfig::default())
    }

    pub fn open_local_with_user_config(
        path: &str,
        db: &Path,
        cache: &Path,
        user: UserConfig,
    ) -> crate::Result<App> {
        App::open(
            path,
            &AppConfig {
                cache_dir: cache.to_path_buf(),
                db_path: db.to_path_buf(),
                refresh: false,
                no_fx: true,
                user,
                config_path: cache.join("test-config.toml"),
            },
        )
    }

    pub fn place_name(app: &App) -> &'static str {
        match app.place {
            Place::Home => "home",
            Place::Tree => "tree",
            Place::Typing => "typing",
            Place::Result => "result",
        }
    }

    pub fn overlay_name(app: &App) -> Option<&'static str> {
        match app.overlay {
            Some(Overlay::Search) => Some("search"),
            Some(Overlay::Settings) => Some("settings"),
            Some(Overlay::Stats) => Some("stats"),
            Some(Overlay::Pause) => Some("pause"),
            None => None,
        }
    }

    pub fn help_open(app: &App) -> bool {
        app.help
    }

    pub fn press(app: &mut App, code: KeyCode) -> crate::Result<bool> {
        press_with(app, code, KeyModifiers::NONE)
    }

    pub fn press_char(app: &mut App, c: char) -> crate::Result<bool> {
        press_with(app, KeyCode::Char(c), KeyModifiers::NONE)
    }

    pub fn press_with(
        app: &mut App,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> crate::Result<bool> {
        app.handle_key(KeyEvent::new(code, modifiers))
    }

    pub fn complete_recommended(app: &mut App) -> crate::Result<TypingMetrics> {
        let path = app
            .progress()
            .recommend_path()
            .ok_or(Error::NoChunks)?
            .to_string();
        app.start_file(&path, 0)?;
        if app
            .session
            .as_ref()
            .and_then(|s| s.engine.as_ref())
            .is_none()
        {
            return Ok(app
                .session
                .as_ref()
                .and_then(|s| s.result.as_ref())
                .map(|r| r.metrics.clone())
                .unwrap_or_else(|| TypingMetrics::from_counts(0, 0, 0)));
        }
        if let Some(session) = &mut app.session {
            if let Some(engine) = &mut session.engine {
                loop {
                    let snap = engine.snapshot();
                    if snap.state != SessionState::Active {
                        break;
                    }
                    match snap.expected {
                        Some('\n') => engine.apply(TypingCommand::Enter),
                        Some(c) => engine.apply(TypingCommand::Char(c)),
                        None => break,
                    }
                }
            }
        }
        let completed = app
            .session
            .as_ref()
            .and_then(|s| s.engine.as_ref())
            .map(|e| e.snapshot().state == SessionState::Completed)
            .unwrap_or(true);
        app.finish_typing(completed)?;
        Ok(app
            .session
            .as_ref()
            .and_then(|s| s.result.as_ref())
            .map(|r| r.metrics.clone())
            .unwrap_or_else(|| TypingMetrics::from_counts(0, 0, 0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn home_stays_until_open() {
        let dir = tempdir().unwrap();
        let cfg = AppConfig {
            cache_dir: dir.path().join("cache"),
            db_path: dir.path().join("db.sqlite"),
            refresh: false,
            no_fx: true,
            user: UserConfig::default(),
            config_path: dir.path().join("config.toml"),
        };
        let app = App::home(&cfg).unwrap();
        assert_eq!(headless::place_name(&app), "home");
        assert!(app.session.is_none());
    }

    fn test_cfg(dir: &std::path::Path) -> AppConfig {
        AppConfig {
            cache_dir: dir.join("cache"),
            db_path: dir.join("db.sqlite"),
            refresh: false,
            no_fx: true,
            user: UserConfig::default(),
            config_path: dir.join("config.toml"),
        }
    }

    #[test]
    fn q_from_tree_returns_home_instead_of_quit() {
        let dir = tempdir().unwrap();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample_repo");
        let mut app = headless::open_local(
            fixture.to_str().unwrap(),
            &dir.path().join("db.sqlite"),
            &dir.path().join("cache"),
        )
        .unwrap();
        assert_eq!(headless::place_name(&app), "tree");
        assert!(!headless::press_char(&mut app, 'q').unwrap());
        assert_eq!(headless::place_name(&app), "home");
        assert!(headless::press_char(&mut app, 'q').unwrap());
    }

    #[test]
    fn settings_overlay_keeps_tree_session() {
        let dir = tempdir().unwrap();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample_repo");
        let mut app = headless::open_local(
            fixture.to_str().unwrap(),
            &dir.path().join("db.sqlite"),
            &dir.path().join("cache"),
        )
        .unwrap();
        headless::press_char(&mut app, ',').unwrap();
        assert_eq!(headless::overlay_name(&app), Some("settings"));
        assert_eq!(headless::place_name(&app), "tree");
        assert!(app.session.is_some());
        headless::press(&mut app, KeyCode::Esc).unwrap();
        assert_eq!(headless::overlay_name(&app), None);
        assert_eq!(headless::place_name(&app), "tree");
    }

    #[test]
    fn search_j_is_always_query_text() {
        let dir = tempdir().unwrap();
        let mut app = App::home(&test_cfg(dir.path())).unwrap();
        headless::press_char(&mut app, '/').unwrap();
        assert_eq!(headless::overlay_name(&app), Some("search"));
        headless::press_char(&mut app, 'j').unwrap();
        headless::press_char(&mut app, 'k').unwrap();
        assert_eq!(app.search.query, "jk");
    }

    #[test]
    fn typing_esc_pauses_and_t_interrupts_without_result() {
        let dir = tempdir().unwrap();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample_repo");
        let mut app = headless::open_local(
            fixture.to_str().unwrap(),
            &dir.path().join("db.sqlite"),
            &dir.path().join("cache"),
        )
        .unwrap();
        let path = app.progress().recommend_path().unwrap().to_string();
        app.start_file(&path, 0).unwrap();
        assert_eq!(headless::place_name(&app), "typing");
        headless::press(&mut app, KeyCode::Esc).unwrap();
        assert_eq!(headless::overlay_name(&app), Some("pause"));
        headless::press_char(&mut app, 't').unwrap();
        assert_eq!(headless::place_name(&app), "tree");
        assert_eq!(headless::overlay_name(&app), None);
        assert!(app.session.as_ref().is_some_and(|s| s.result.is_none()));
        assert_eq!(
            app.progress()
                .files
                .iter()
                .find(|f| f.relative_path == path)
                .map(crate::domain::content::FileProgress::derive_status),
            Some(FileStatus::Todo)
        );
    }

    #[test]
    fn typing_checkpoint_resumes_and_restart_clears_it() {
        let dir = tempdir().unwrap();
        let repo_dir = dir.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        std::fs::write(repo_dir.join("main.rs"), "one\ntwo\nthree").unwrap();
        let db = dir.path().join("db.sqlite");
        let cache = dir.path().join("cache");

        let mut app = headless::open_local(repo_dir.to_str().unwrap(), &db, &cache).unwrap();
        let path = app.progress().recommend_path().unwrap().to_string();
        app.start_file(&path, 0).unwrap();
        headless::press_char(&mut app, 'x').unwrap();
        headless::press_char(&mut app, 'o').unwrap();
        headless::press_char(&mut app, 'n').unwrap();
        headless::press_char(&mut app, 'e').unwrap();
        headless::press(&mut app, KeyCode::Enter).unwrap();
        headless::press(&mut app, KeyCode::Esc).unwrap();
        headless::press_char(&mut app, 't').unwrap();
        assert_eq!(app.progress().completed_lines(), 1);

        let mut resumed = headless::open_local(repo_dir.to_str().unwrap(), &db, &cache).unwrap();
        resumed.start_file(&path, 0).unwrap();
        let snapshot = resumed
            .session
            .as_ref()
            .and_then(|session| session.engine.as_ref())
            .expect("resumed engine")
            .snapshot();
        assert_eq!(snapshot.cursor, 4);
        assert_eq!(snapshot.keystrokes, 4);
        assert_eq!(snapshot.misses, 1);

        headless::press(&mut resumed, KeyCode::Esc).unwrap();
        headless::press_char(&mut resumed, 'r').unwrap();
        let restarted = resumed
            .session
            .as_ref()
            .and_then(|session| session.engine.as_ref())
            .expect("restarted engine")
            .snapshot();
        assert_eq!(restarted.cursor, 0);
        assert_eq!(restarted.keystrokes, 0);
        assert_eq!(restarted.misses, 0);
        assert!(resumed
            .progress()
            .files
            .iter()
            .flat_map(|file| file.chunks.iter())
            .all(|chunk| chunk.checkpoint.is_none()));

        headless::press_char(&mut resumed, 'o').unwrap();
        assert!(
            headless::press_with(&mut resumed, KeyCode::Char('c'), KeyModifiers::CONTROL,).unwrap()
        );
        let mut ctrl_c_resumed =
            headless::open_local(repo_dir.to_str().unwrap(), &db, &cache).unwrap();
        ctrl_c_resumed.start_file(&path, 0).unwrap();
        assert_eq!(
            ctrl_c_resumed
                .session
                .as_ref()
                .and_then(|session| session.engine.as_ref())
                .expect("Ctrl-C checkpoint")
                .snapshot()
                .cursor,
            1
        );
    }

    #[test]
    fn result_enter_starts_next_and_r_retries() {
        let dir = tempdir().unwrap();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample_repo");
        let mut app = headless::open_local(
            fixture.to_str().unwrap(),
            &dir.path().join("db.sqlite"),
            &dir.path().join("cache"),
        )
        .unwrap();
        let first = app.progress().recommend_path().unwrap().to_string();
        headless::complete_recommended(&mut app).unwrap();
        assert_eq!(headless::place_name(&app), "result");
        let next = app
            .session
            .as_ref()
            .and_then(|s| s.tree.recommend.clone())
            .expect("next recommendation");
        assert_ne!(next, first);
        headless::press(&mut app, KeyCode::Enter).unwrap();
        assert_eq!(headless::place_name(&app), "typing");
        assert_eq!(
            app.session.as_ref().map(|s| s.typing_path.as_str()),
            Some(next.as_str())
        );
        headless::press(&mut app, KeyCode::Esc).unwrap();
        headless::press_char(&mut app, 't').unwrap();
        app.start_file(&first, 0).unwrap();
        // empty file bodies complete immediately
        if headless::place_name(&app) == "result" {
            headless::press_char(&mut app, 'r').unwrap();
            assert_eq!(
                app.session.as_ref().map(|s| s.typing_path.as_str()),
                Some(first.as_str())
            );
        }
    }

    #[test]
    fn help_opens_from_tree() {
        let dir = tempdir().unwrap();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample_repo");
        let mut app = headless::open_local(
            fixture.to_str().unwrap(),
            &dir.path().join("db.sqlite"),
            &dir.path().join("cache"),
        )
        .unwrap();
        headless::press_char(&mut app, '?').unwrap();
        assert!(headless::help_open(&app));
        headless::press(&mut app, KeyCode::Esc).unwrap();
        assert!(!headless::help_open(&app));
    }

    #[test]
    fn tree_tab_jumps_to_recommend() {
        let dir = tempdir().unwrap();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample_repo");
        let mut app = headless::open_local(
            fixture.to_str().unwrap(),
            &dir.path().join("db.sqlite"),
            &dir.path().join("cache"),
        )
        .unwrap();
        if let Some(s) = &mut app.session {
            s.tree.select_last();
        }
        headless::press(&mut app, KeyCode::Tab).unwrap();
        let selected = app
            .session
            .as_ref()
            .and_then(|s| s.tree.selected_file_path());
        let recommend = app.session.as_ref().and_then(|s| s.tree.recommend.clone());
        assert_eq!(selected, recommend);
    }

    #[test]
    fn tree_x_skips_and_x_clears_after_refresh() {
        let dir = tempdir().unwrap();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample_repo");
        let db = dir.path().join("db.sqlite");
        let cache = dir.path().join("cache");
        let mut app = headless::open_local(fixture.to_str().unwrap(), &db, &cache).unwrap();
        let path = app.progress().recommend_path().unwrap().to_string();
        assert!(
            app.session
                .as_mut()
                .is_some_and(|s| s.tree.jump_to_path(&path)),
            "first jump to {path}"
        );
        headless::press_char(&mut app, 'x').unwrap();
        assert_eq!(
            app.progress()
                .files
                .iter()
                .find(|f| f.relative_path == path)
                .map(|f| f.derive_status()),
            Some(FileStatus::Skipped)
        );
        let mut app2 = headless::open_local(fixture.to_str().unwrap(), &db, &cache).unwrap();
        assert_eq!(
            app2.progress()
                .files
                .iter()
                .find(|f| f.relative_path == path)
                .and_then(|f| f.manual_override),
            Some(ManualOverride::Skip)
        );
        assert!(
            app2.session
                .as_mut()
                .is_some_and(|s| s.tree.jump_to_path(&path)),
            "second jump to {path}"
        );
        headless::press_char(&mut app2, 'X').unwrap();
        assert_eq!(
            app2.progress()
                .files
                .iter()
                .find(|f| f.relative_path == path)
                .map(|f| f.derive_status()),
            Some(FileStatus::Todo)
        );
    }
}
