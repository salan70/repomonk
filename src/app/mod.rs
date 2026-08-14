//! Application state machine: places (Home / Tree / Typing / Result) plus overlays.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::Color;

use crate::config::{save as save_user_config, UserConfig};
use crate::config::{DependencyDirection, ProgressMode};
use crate::domain::content::{
    ChunkCompletion, FileStatus, ImportEdge, ManualOverride, RepoProgress, ResolvedRepository,
    SessionSummary, TypingCheckpoint, TypingMetrics,
};
use crate::domain::dependency::{order_files, uses_flow_mode, FlowOrder};
use crate::domain::entry::{detect_entry_candidates, EntryCandidate};
use crate::domain::file_type::{hidden_paths, FileTypePrefs, FileTypeState};
use crate::domain::typing::{SessionState, TypingCommand, TypingEngine};
use crate::scan::extract::ExtractOptions;
use crate::scan::walk::{scan_repository, single_file_scan, WalkOptions};
use crate::source::resolve_source;
use crate::store::SqliteStore;
use crate::ui::file_types::{draw_file_types, FileTypesView};
use crate::ui::flow::{draw_flow, FlowView};
use crate::ui::fx::FxState;
use crate::ui::help::{draw_help, HelpContext};
use crate::ui::highlight::highlight_chars;
use crate::ui::home::{draw_home, draw_search_modal, HomeView};
use crate::ui::pause::draw_pause;
use crate::ui::result::{draw_result, NextStep, ResultView};
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
    FileTypes,
    Flow,
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
    import_edges: Vec<ImportEdge>,
    dependency_direction: DependencyDirection,
    progress_mode: ProgressMode,
    entry: Option<String>,
    flow: Option<FlowOrder>,
    manifest_hints: Vec<EntryCandidate>,
    /// True right after `load_session` when saved file-type prefs were empty
    /// (i.e. this repository has never had the File types overlay closed on it).
    show_file_types_overlay: bool,
    /// True when this repository has never chosen a progress mode.
    show_flow_overlay: bool,
}

struct DependencySettings {
    import_edges: Vec<ImportEdge>,
    mode: ProgressMode,
    direction: DependencyDirection,
    entry: Option<String>,
    manifest_hints: Vec<EntryCandidate>,
    show_flow_overlay: bool,
}

/// Tree-display flags that come from user config plus the current file-type prefs.
struct DisplaySettings {
    hide_skipped: bool,
    show_file_types_overlay: bool,
    hidden_paths: std::collections::HashSet<String>,
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
    file_types: Option<FileTypesView>,
    flow_view: Option<FlowView>,
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
        let show_file_types = session.show_file_types_overlay;
        let show_flow = session.show_flow_overlay;
        let mut app = Self {
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
            file_types: None,
            flow_view: None,
            session: Some(session),
            fx: FxState::with_config(cfg.user.fx.intensity, cfg.user.fx.preset),
            splash_started: None,
        };
        if show_file_types {
            app.open_file_types();
        } else if show_flow {
            app.open_flow();
        }
        Ok(app)
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
            file_types: None,
            flow_view: None,
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
                                    let step_label = s.flow.as_ref().and_then(|order| {
                                        order
                                            .step_number(&s.typing_path)
                                            .map(|n| format!("{n}/{}", order.reachable_total()))
                                    });
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
                                        step_label.as_deref(),
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
                            Some(Overlay::FileTypes) => {
                                if let Some(view) = &self.file_types {
                                    draw_file_types(frame, area, view);
                                }
                            }
                            Some(Overlay::Flow) => {
                                if let Some(view) = &self.flow_view {
                                    draw_flow(frame, area, view);
                                }
                            }
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
                    if self.handle_key_event(key)? {
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

    /// Dispatches key presses and repeats, ignoring release notifications.
    fn handle_key_event(&mut self, key: KeyEvent) -> crate::Result<bool> {
        if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            self.handle_key(key)
        } else {
            Ok(false)
        }
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
            Some(Overlay::FileTypes) => return self.handle_file_types_key(key),
            Some(Overlay::Flow) => return self.handle_flow_key(key),
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
                    Overlay::FileTypes => HelpContext::FileTypes,
                    Overlay::Flow => HelpContext::Flow,
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

    fn open_file_types(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        let saved = self
            .store
            .find_repository_id(&session.repo.identity)
            .ok()
            .flatten()
            .map(|id| self.store.load_file_type_prefs(id).unwrap_or_default())
            .unwrap_or_default();
        self.file_types = Some(FileTypesView::from_progress(
            &session.repo.display_name,
            &session.progress,
            &saved,
        ));
        self.overlay = Some(Overlay::FileTypes);
    }

    fn open_flow(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        if session.single_file.is_some() {
            return;
        }
        let typeable: Vec<String> = session
            .progress
            .files
            .iter()
            .filter(|file| file.derive_status() != FileStatus::Skipped)
            .map(|file| file.relative_path.clone())
            .collect();
        let candidates =
            detect_entry_candidates(&typeable, &session.import_edges, &session.manifest_hints);
        let flow_enabled =
            !session.import_edges.is_empty() && !candidates.is_empty() && typeable.len() > 1;
        let disabled_reason = if !flow_enabled {
            if session.import_edges.is_empty() {
                Some("no import-analyzable language found".into())
            } else if candidates.is_empty() {
                Some("no entry point could be detected".into())
            } else {
                Some("need more than one typeable file".into())
            }
        } else {
            None
        };
        let tree_selection = session.tree.selected_file_path().filter(|path| {
            session.progress.files.iter().any(|file| {
                file.relative_path == *path && file.derive_status() != FileStatus::Skipped
            })
        });
        let file_count = session
            .flow
            .as_ref()
            .map(FlowOrder::reachable_total)
            .unwrap_or(typeable.len());
        self.flow_view = Some(FlowView::new(
            &session.repo.display_name,
            session.progress_mode,
            session.entry.clone(),
            candidates,
            tree_selection,
            flow_enabled,
            disabled_reason,
            file_count,
        ));
        self.overlay = Some(Overlay::Flow);
    }

    fn handle_file_types_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('t') => {
                self.close_overlay()?;
                Ok(false)
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(view) = &mut self.file_types {
                    view.move_by(1);
                }
                Ok(false)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(view) = &mut self.file_types {
                    view.move_by(-1);
                }
                Ok(false)
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                if let Some(view) = &mut self.file_types {
                    view.cycle_selected();
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_flow_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('e') => {
                self.close_overlay()?;
                Ok(false)
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(view) = &mut self.flow_view {
                    view.move_by(1);
                }
                Ok(false)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(view) = &mut self.flow_view {
                    view.move_by(-1);
                }
                Ok(false)
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                if let Some(view) = &mut self.flow_view {
                    view.activate_selected();
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    /// Persist the edited file-type toggles and rescan. On failure (e.g. every
    /// type disabled), the previous prefs and session are kept and an error is shown.
    fn apply_file_type_prefs(&mut self) -> crate::Result<()> {
        let Some(view) = self.file_types.take() else {
            return Ok(());
        };
        let Some(session) = &self.session else {
            return Ok(());
        };
        let repo_id = session.repo_id;
        let input = session.repo.input.clone();
        let selected_path = session.tree.selected_path();

        let old_prefs: Vec<(String, FileTypeState)> = self
            .store
            .load_file_type_prefs(repo_id)
            .unwrap_or_default()
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();

        let new_prefs = view.to_prefs();
        self.store.save_file_type_prefs(repo_id, &new_prefs)?;

        match load_session(&input, &self.cfg, &mut self.store) {
            Ok(mut new_session) => {
                if let Some(path) = &selected_path {
                    new_session.tree.jump_to_path(path);
                }
                self.session = Some(new_session);
            }
            Err(err) => {
                // Revert the saved prefs so the overlay does not force itself
                // open again next time, and keep the existing session intact.
                let _ = self.store.save_file_type_prefs(repo_id, &old_prefs);
                if let Some(session) = &mut self.session {
                    session.tree.message = Some(format!("file types not applied: {err}"));
                }
            }
        }
        Ok(())
    }

    fn close_overlay(&mut self) -> crate::Result<()> {
        if self.overlay == Some(Overlay::Settings) {
            self.apply_live_settings()?;
        }
        if self.overlay == Some(Overlay::FileTypes) {
            self.apply_file_type_prefs()?;
            self.overlay = None;
            if self.should_prompt_flow() {
                self.open_flow();
            }
            return Ok(());
        }
        if self.overlay == Some(Overlay::Flow) {
            self.apply_flow_prefs()?;
        }
        self.overlay = None;
        Ok(())
    }

    fn should_prompt_flow(&self) -> bool {
        let Some(session) = &self.session else {
            return false;
        };
        if session.single_file.is_some() {
            return false;
        }
        self.store
            .load_repo_flow_prefs(session.repo_id)
            .ok()
            .is_some_and(|(mode, _)| mode.is_none())
    }

    fn apply_flow_prefs(&mut self) -> crate::Result<()> {
        let Some(view) = self.flow_view.take() else {
            return Ok(());
        };
        let Some(session) = &self.session else {
            return Ok(());
        };
        let repo_id = session.repo_id;
        let mode = if view.flow_enabled {
            view.mode
        } else {
            ProgressMode::Manual
        };
        let entry = view.entry;
        self.store
            .save_repo_flow_prefs(repo_id, mode, entry.as_deref())?;
        if let Some(session) = &mut self.session {
            session.progress_mode = mode;
            session.entry = entry;
            session.recompute_flow();
        }
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
            KeyCode::Char('t') => {
                self.open_file_types();
                Ok(false)
            }
            KeyCode::Char('e') => {
                self.open_flow();
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
        session.recompute_flow();
        Ok(())
    }

    fn open_from_home(&mut self, input: &str) -> crate::Result<()> {
        match load_session(input, &self.cfg, &mut self.store) {
            Ok(session) => {
                let show_file_types = session.show_file_types_overlay;
                let show_flow = session.show_flow_overlay;
                self.session = Some(session);
                self.home.error = None;
                self.place = Place::Tree;
                self.overlay = None;
                self.help = false;
                self.showing_splash = false;
                self.splash_started = None;
                if show_file_types {
                    self.open_file_types();
                } else if show_flow {
                    self.open_flow();
                }
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
            let next = session.flow.as_ref().and_then(|order| {
                let step = order.next_step(&session.progress)?;
                Some(NextStep {
                    index: order.step_number(&step.path)?,
                    total: order.reachable_total(),
                    path: step.path.clone(),
                    via: step.via.clone(),
                })
            });
            session.result = Some(ResultView {
                repo: session.repo.display_name.clone(),
                path: typing_path,
                completed,
                metrics,
                file_done,
                next,
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

    if resolved.root.is_dir() && resolved.identity.starts_with("local:") {
        let local_path = resolved.identity.trim_start_matches("local:");
        let p = PathBuf::from(local_path);
        if p.is_file() {
            // Explicitly opening a single file bypasses saved repository-level prefs.
            let walk = walk_options(&cfg.user, FileTypePrefs::default());
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
                    mode: ProgressMode::Manual,
                    direction: cfg.user.progress.dependency_direction,
                    entry: None,
                    manifest_hints: Vec::new(),
                    show_flow_overlay: false,
                },
                DisplaySettings {
                    hide_skipped: cfg.user.progress.hide_skipped,
                    show_file_types_overlay: false,
                    hidden_paths: std::collections::HashSet::new(),
                },
            ));
        }
    }

    let saved_prefs = match store.find_repository_id(&resolved.identity)? {
        Some(id) => store.load_file_type_prefs(id)?,
        None => FileTypePrefs::default(),
    };
    let show_file_types_overlay = saved_prefs.is_empty();
    let walk = walk_options(&cfg.user, saved_prefs.clone());

    let (progress_repo, scan, single_file) = if resolved.root.is_dir() {
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
    let hidden = hidden_paths(&progress, &saved_prefs);
    let import_edges = scan.import_edges.clone();
    let (saved_mode, saved_entry) = store.load_repo_flow_prefs(repo_id)?;
    let show_flow_overlay = saved_mode.is_none();
    let mode = saved_mode.unwrap_or(cfg.user.progress.mode);
    let manifest_hints = crate::scan::manifest::read_entry_hints(&progress_repo.root);
    Ok(RepoSession::from_parts(
        progress_repo,
        repo_id,
        progress,
        single_file,
        DependencySettings {
            import_edges,
            mode,
            direction: cfg.user.progress.dependency_direction,
            entry: saved_entry,
            manifest_hints,
            show_flow_overlay,
        },
        DisplaySettings {
            hide_skipped: cfg.user.progress.hide_skipped,
            show_file_types_overlay,
            hidden_paths: hidden,
        },
    ))
}

fn walk_options(user: &UserConfig, file_types: FileTypePrefs) -> WalkOptions {
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
        file_types,
    }
}

impl RepoSession {
    fn from_parts(
        repo: ResolvedRepository,
        repo_id: i64,
        progress: RepoProgress,
        single_file: Option<String>,
        dependency: DependencySettings,
        display: DisplaySettings,
    ) -> Self {
        let typeable_paths = typeable_paths(&progress);
        let candidates = detect_entry_candidates(
            &typeable_paths,
            &dependency.import_edges,
            &dependency.manifest_hints,
        );
        let mut entry_message = None;
        let saved_entry = dependency.entry.clone();
        let entry = saved_entry
            .as_ref()
            .filter(|path| typeable_paths.iter().any(|item| item == *path))
            .cloned()
            .or_else(|| {
                if saved_entry.is_some() {
                    entry_message =
                        Some("saved entry is no longer typeable; using detected entry".into());
                }
                candidates.first().map(|candidate| candidate.path.clone())
            });
        let flow_active = uses_flow_mode(dependency.mode)
            && !dependency.import_edges.is_empty()
            && typeable_paths.len() > 1;
        let flow = flow_active.then(|| {
            let steps = order_files(
                &dependency.import_edges,
                entry.as_deref(),
                dependency.direction,
                &typeable_paths,
            );
            FlowOrder::new(
                entry
                    .clone()
                    .unwrap_or_else(|| typeable_paths.first().cloned().unwrap_or_default()),
                steps,
            )
        });
        let show_file_types_overlay = display.show_file_types_overlay;
        let mut tree = TreeView::from_progress_full(
            &repo.display_name,
            &progress,
            display.hide_skipped,
            flow.clone(),
            display.hidden_paths,
        );
        if let Some(message) = entry_message {
            tree.message = Some(message);
        }
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
            progress_mode: dependency.mode,
            entry,
            flow,
            manifest_hints: dependency.manifest_hints,
            show_file_types_overlay,
            show_flow_overlay: dependency.show_flow_overlay,
        }
    }

    fn recompute_flow(&mut self) {
        let typeable_paths = typeable_paths(&self.progress);
        let flow_active = uses_flow_mode(self.progress_mode)
            && !self.import_edges.is_empty()
            && typeable_paths.len() > 1;
        self.flow = if flow_active {
            let steps = order_files(
                &self.import_edges,
                self.entry.as_deref(),
                self.dependency_direction,
                &typeable_paths,
            );
            Some(FlowOrder::new(
                self.entry
                    .clone()
                    .unwrap_or_else(|| typeable_paths.first().cloned().unwrap_or_default()),
                steps,
            ))
        } else {
            None
        };
        self.tree.set_flow(&self.progress, self.flow.clone());
    }
}

fn typeable_paths(progress: &RepoProgress) -> Vec<String> {
    let mut paths: Vec<String> = progress
        .files
        .iter()
        .filter(|file| file.derive_status() != FileStatus::Skipped)
        .map(|file| file.relative_path.clone())
        .collect();
    paths.sort();
    paths.dedup();
    paths
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
        let mut app = App::open(
            path,
            &AppConfig {
                cache_dir: cache.to_path_buf(),
                db_path: db.to_path_buf(),
                refresh: false,
                no_fx: true,
                user,
                config_path: cache.join("test-config.toml"),
            },
        )?;
        // First-ever open shows File types then the flow dialog; accept the
        // defaults so callers that are not exercising those overlays see Tree.
        if overlay_name(&app) == Some("file_types") {
            press(&mut app, KeyCode::Esc)?;
        }
        if overlay_name(&app) == Some("flow") {
            press(&mut app, KeyCode::Esc)?;
        }
        Ok(app)
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
            Some(Overlay::FileTypes) => Some("file_types"),
            Some(Overlay::Flow) => Some("flow"),
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

    pub fn recommend_path(app: &App) -> Option<String> {
        app.session.as_ref().and_then(|s| s.tree.recommend.clone())
    }

    pub fn flow_paths(app: &App) -> Option<Vec<String>> {
        app.session.as_ref().and_then(|s| {
            s.flow
                .as_ref()
                .map(|order| order.steps.iter().map(|step| step.path.clone()).collect())
        })
    }

    pub fn flow_entry(app: &App) -> Option<String> {
        app.session
            .as_ref()
            .and_then(|s| s.entry.clone())
            .or_else(|| {
                app.session
                    .as_ref()
                    .and_then(|s| s.flow.as_ref().map(|order| order.entry.clone()))
            })
    }

    pub fn complete_recommended(app: &mut App) -> crate::Result<TypingMetrics> {
        let path = app
            .session
            .as_ref()
            .and_then(|s| s.tree.recommend.clone())
            .or_else(|| app.progress().recommend_path().map(str::to_string))
            .ok_or(Error::NoChunks)?;
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
    fn tree_j_and_k_move_on_key_repeat() {
        let dir = tempdir().unwrap();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample_repo");
        let mut app = headless::open_local(
            fixture.to_str().unwrap(),
            &dir.path().join("db.sqlite"),
            &dir.path().join("cache"),
        )
        .unwrap();

        let start = app
            .session
            .as_ref()
            .and_then(|s| s.tree.selected_path())
            .unwrap();

        app.handle_key_event(KeyEvent::new_with_kind(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ))
        .unwrap();
        let first = app
            .session
            .as_ref()
            .and_then(|s| s.tree.selected_path())
            .unwrap();
        assert_ne!(first, start);

        app.handle_key_event(KeyEvent::new_with_kind(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        ))
        .unwrap();
        let second = app
            .session
            .as_ref()
            .and_then(|s| s.tree.selected_path())
            .unwrap();
        assert_ne!(second, first);

        app.handle_key_event(KeyEvent::new_with_kind(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        ))
        .unwrap();
        assert_eq!(
            app.session
                .as_ref()
                .and_then(|s| s.tree.selected_path())
                .as_deref(),
            Some(first.as_str())
        );
    }

    #[test]
    fn tree_x_skips_and_x_clears_after_refresh() {
        let dir = tempdir().unwrap();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample_repo");
        let db = dir.path().join("db.sqlite");
        let cache = dir.path().join("cache");
        let mut user = UserConfig::default();
        user.progress.hide_skipped = false;
        let mut app =
            headless::open_local_with_user_config(fixture.to_str().unwrap(), &db, &cache, user)
                .unwrap();
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
        let mut user = UserConfig::default();
        user.progress.hide_skipped = false;
        let mut app2 =
            headless::open_local_with_user_config(fixture.to_str().unwrap(), &db, &cache, user)
                .unwrap();
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

    fn types_repo_dir() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "documentation\n").unwrap();
        dir
    }

    #[test]
    fn hiding_file_type_removes_files_and_rolls_up_empty_dirs_from_tree() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("docs/a.md"), "a\n").unwrap();
        std::fs::write(dir.path().join("docs/b.md"), "b\n").unwrap();
        let state_dir = tempdir().unwrap();
        let db = state_dir.path().join("db.sqlite");
        let cache = state_dir.path().join("cache");
        let mut app = headless::open_local(dir.path().to_str().unwrap(), &db, &cache).unwrap();

        headless::press_char(&mut app, 't').unwrap();
        let md_idx = app
            .file_types
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .position(|e| e.key == ".md")
            .unwrap();
        app.file_types.as_mut().unwrap().selected = md_idx;
        headless::press_char(&mut app, ' ').unwrap();
        assert_eq!(
            app.file_types.as_ref().unwrap().entries[md_idx].state,
            crate::domain::file_type::FileTypeState::Hidden
        );
        headless::press(&mut app, KeyCode::Esc).unwrap();

        let paths: Vec<_> = app
            .session
            .as_ref()
            .unwrap()
            .tree
            .rows
            .iter()
            .map(|r| r.name.clone())
            .collect();
        assert!(!paths.contains(&"docs".to_string()));
        assert!(!paths.contains(&"a.md".to_string()));
        assert!(paths.contains(&"main.rs".to_string()));
    }

    #[test]
    fn file_types_overlay_opens_on_first_open_only() {
        let repo = types_repo_dir();
        let state_dir = tempdir().unwrap();
        let db = state_dir.path().join("db.sqlite");
        let cache = state_dir.path().join("cache");

        // Do not use the headless helper here: it auto-dismisses the overlay.
        let mut app = App::open(
            repo.path().to_str().unwrap(),
            &AppConfig {
                cache_dir: cache.clone(),
                db_path: db.clone(),
                refresh: false,
                no_fx: true,
                user: UserConfig::default(),
                config_path: cache.join("test-config.toml"),
            },
        )
        .unwrap();
        assert_eq!(headless::overlay_name(&app), Some("file_types"));
        headless::press(&mut app, KeyCode::Esc).unwrap();
        assert_eq!(headless::overlay_name(&app), Some("flow"));
        headless::press(&mut app, KeyCode::Esc).unwrap();
        assert_eq!(headless::overlay_name(&app), None);

        let app2 = App::open(
            repo.path().to_str().unwrap(),
            &AppConfig {
                cache_dir: cache,
                db_path: db,
                refresh: false,
                no_fx: true,
                user: UserConfig::default(),
                config_path: PathBuf::from("test-config.toml"),
            },
        )
        .unwrap();
        assert_eq!(headless::overlay_name(&app2), None);
    }

    #[test]
    fn tree_t_opens_file_types_overlay_and_enabling_md_makes_readme_todo() {
        let repo = types_repo_dir();
        let state_dir = tempdir().unwrap();
        let db = state_dir.path().join("db.sqlite");
        let cache = state_dir.path().join("cache");
        let mut app = headless::open_local(repo.path().to_str().unwrap(), &db, &cache).unwrap();

        assert_eq!(
            app.progress()
                .files
                .iter()
                .find(|f| f.relative_path == "README.md")
                .map(|f| f.derive_status()),
            Some(FileStatus::Skipped)
        );

        headless::press_char(&mut app, 't').unwrap();
        assert_eq!(headless::overlay_name(&app), Some("file_types"));
        assert!(app
            .file_types
            .as_ref()
            .is_some_and(|v| v.entries.iter().any(|e| e.key == ".md")));
        let md_idx = app
            .file_types
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .position(|e| e.key == ".md")
            .unwrap();
        app.file_types.as_mut().unwrap().selected = md_idx;
        // Default is Excluded; cycle Excluded -> Hidden -> Included.
        headless::press_char(&mut app, ' ').unwrap();
        assert_eq!(
            app.file_types.as_ref().unwrap().entries[md_idx].state,
            crate::domain::file_type::FileTypeState::Hidden
        );
        headless::press_char(&mut app, ' ').unwrap();
        assert_eq!(
            app.file_types.as_ref().unwrap().entries[md_idx].state,
            crate::domain::file_type::FileTypeState::Included
        );
        headless::press(&mut app, KeyCode::Esc).unwrap();

        assert_eq!(headless::overlay_name(&app), None);
        assert_eq!(headless::place_name(&app), "tree");
        assert_eq!(
            app.progress()
                .files
                .iter()
                .find(|f| f.relative_path == "README.md")
                .map(|f| f.derive_status()),
            Some(FileStatus::Todo)
        );
    }

    #[test]
    fn disabling_all_file_types_keeps_previous_session_and_shows_error() {
        let repo = types_repo_dir();
        let state_dir = tempdir().unwrap();
        let db = state_dir.path().join("db.sqlite");
        let cache = state_dir.path().join("cache");
        let mut app = headless::open_local(repo.path().to_str().unwrap(), &db, &cache).unwrap();
        let before = app.progress().clone();

        headless::press_char(&mut app, 't').unwrap();
        let rs_idx = app
            .file_types
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .position(|e| e.key == ".rs")
            .unwrap();
        app.file_types.as_mut().unwrap().selected = rs_idx;
        headless::press_char(&mut app, ' ').unwrap();
        headless::press(&mut app, KeyCode::Esc).unwrap();

        assert_eq!(headless::overlay_name(&app), None);
        assert_eq!(headless::place_name(&app), "tree");
        assert_eq!(app.progress(), &before);
        assert!(app
            .session
            .as_ref()
            .and_then(|s| s.tree.message.as_ref())
            .is_some());
    }

    #[test]
    fn file_type_toggle_rescan_preserves_done_state() {
        let repo = types_repo_dir();
        let state_dir = tempdir().unwrap();
        let db = state_dir.path().join("db.sqlite");
        let cache = state_dir.path().join("cache");
        let mut app = headless::open_local(repo.path().to_str().unwrap(), &db, &cache).unwrap();
        headless::complete_recommended(&mut app).unwrap();
        assert_eq!(headless::place_name(&app), "result");
        headless::press_char(&mut app, 't').unwrap();
        assert_eq!(headless::place_name(&app), "tree");
        assert_eq!(headless::overlay_name(&app), None);

        headless::press_char(&mut app, 't').unwrap();
        assert_eq!(headless::overlay_name(&app), Some("file_types"));
        let md_idx = app
            .file_types
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .position(|e| e.key == ".md")
            .unwrap();
        app.file_types.as_mut().unwrap().selected = md_idx;
        headless::press_char(&mut app, ' ').unwrap();
        headless::press_char(&mut app, 'q').unwrap();

        assert_eq!(
            app.progress()
                .files
                .iter()
                .find(|f| f.relative_path == "main.rs")
                .map(|f| f.derive_status()),
            Some(FileStatus::Done)
        );
    }
}
