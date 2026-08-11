//! Application state machine: Home / Search / Stats / Splash → Tree → Typing → Result.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::domain::content::{
    ChunkCompletion, FileStatus, RepoProgress, ResolvedRepository, SessionSummary, TypingMetrics,
};
use crate::domain::typing::{SessionState, TypingCommand, TypingEngine};
use crate::scan::walk::{scan_repository, single_file_scan, WalkOptions};
use crate::source::resolve_source;
use crate::store::SqliteStore;
use crate::ui::fx::FxState;
use crate::ui::home::{draw_home, draw_search_modal, HomeView};
use crate::ui::result::{draw_result, ResultView};
use crate::ui::search::SearchState;
use crate::ui::splash::{draw_splash, SPLASH_TOTAL_MS};
use crate::ui::stats::{draw_stats, StatsView};
use crate::ui::terminal::TerminalGuard;
use crate::ui::tree::{draw_tree, TreeView};
use crate::ui::typing::draw_typing;
use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    Search,
    Stats,
    Splash,
    Tree,
    Typing,
    Result,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub cache_dir: PathBuf,
    pub db_path: PathBuf,
    pub refresh: bool,
    /// Enable visual effects (splash animation, glow, trails).
    pub fx_enabled: bool,
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
    typing_chunk_id: i64,
    session_started_at: String,
    result: Option<ResultView>,
    single_file: Option<String>,
}

pub struct App {
    cfg: AppConfig,
    store: SqliteStore,
    screen: Screen,
    home: HomeView,
    search: SearchState,
    stats: Option<StatsView>,
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
            screen: Screen::Tree,
            home,
            search: SearchState::default(),
            stats: None,
            session: Some(session),
            fx: FxState::new(),
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
            screen: Screen::Home,
            home,
            search: SearchState::default(),
            stats: None,
            session: None,
            fx: FxState::new(),
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
        if self.session.is_some() && self.screen == Screen::Tree && self.cfg.fx_enabled {
            self.screen = Screen::Splash;
            self.splash_started = Some(Instant::now());
        }

        loop {
            let now_ms = now_millis();
            let typing_snap = if self.screen == Screen::Typing {
                self.session
                    .as_ref()
                    .and_then(|s| s.engine.as_ref().map(|e| e.snapshot()))
            } else {
                None
            };
            if self.cfg.fx_enabled {
                if let Some(snap) = &typing_snap {
                    self.fx.observe(snap, now_ms);
                }
            }
            let splash_elapsed = self
                .splash_started
                .map(|t| t.elapsed().as_millis() as u64)
                .unwrap_or(0);
            {
                let term = guard.terminal();
                term.draw(|frame| {
                    let area = frame.area();
                    match self.screen {
                        Screen::Home => draw_home(frame, area, &self.home),
                        Screen::Search => {
                            draw_home(frame, area, &self.home);
                            draw_search_modal(frame, area, &self.search);
                        }
                        Screen::Stats => {
                            if let Some(stats) = &self.stats {
                                draw_stats(frame, area, stats);
                            }
                        }
                        Screen::Splash => {
                            if let Some(s) = &self.session {
                                draw_splash(frame, area, splash_elapsed, &s.repo.display_name);
                            }
                        }
                        Screen::Tree => {
                            if let Some(s) = &self.session {
                                draw_tree(frame, area, &s.tree);
                            }
                        }
                        Screen::Typing => {
                            if let (Some(s), Some(snap)) = (&self.session, &typing_snap) {
                                draw_typing(
                                    frame,
                                    area,
                                    &s.typing_path,
                                    &s.typing_chunk_label,
                                    snap,
                                    now_ms,
                                    self.cfg.fx_enabled.then_some(&self.fx),
                                );
                            }
                        }
                        Screen::Result => {
                            if let Some(view) =
                                self.session.as_ref().and_then(|s| s.result.as_ref())
                            {
                                draw_result(frame, area, view);
                            }
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
                if self.screen == Screen::Splash && splash_elapsed >= SPLASH_TOTAL_MS {
                    self.screen = Screen::Tree;
                }
                let finished = self.session.as_mut().and_then(|session| {
                    let engine = session.engine.as_mut()?;
                    engine.apply(TypingCommand::Tick {
                        now_ms: now_millis(),
                    });
                    let state = engine.snapshot().state;
                    if state == SessionState::Completed || state == SessionState::Interrupted {
                        Some(state == SessionState::Completed)
                    } else {
                        None
                    }
                });
                if let Some(completed) = finished {
                    self.finish_typing(completed)?;
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
            return Ok(true);
        }

        match self.screen {
            Screen::Home => self.handle_home_key(key),
            Screen::Search => self.handle_search_key(key),
            Screen::Stats => self.handle_stats_key(key),
            Screen::Splash => {
                self.screen = Screen::Tree;
                Ok(false)
            }
            Screen::Tree => self.handle_tree_key(key),
            Screen::Typing => self.handle_typing_key(key),
            Screen::Result => self.handle_result_key(key),
        }
    }

    fn handle_home_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Char('q') => Ok(true),
            KeyCode::Char('j') | KeyCode::Down => {
                self.home.move_by(1);
                Ok(false)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.home.move_by(-1);
                Ok(false)
            }
            KeyCode::Enter => {
                if let Some(input) = self.home.selected_input().map(str::to_string) {
                    self.open_from_home(&input)?;
                }
                Ok(false)
            }
            KeyCode::Char('s') => {
                self.search = SearchState::default();
                self.search.refresh(&self.home.recent, &self.cfg.cache_dir);
                self.screen = Screen::Search;
                Ok(false)
            }
            KeyCode::Char('g') => {
                self.reload_home_data()?;
                self.stats = Some(StatsView::new(
                    self.home.summary.clone(),
                    self.home.recent.clone(),
                ));
                self.screen = Screen::Stats;
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.screen = Screen::Home;
                Ok(false)
            }
            KeyCode::Enter => {
                if let Some(input) = self.search.confirm_input() {
                    self.open_from_home(&input)?;
                }
                Ok(false)
            }
            KeyCode::Char('j') | KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.move_by(1);
                Ok(false)
            }
            KeyCode::Char('k') | KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
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
                // j/k navigate when not typing? Spec uses j/k for select in search.
                // Prefer character insertion; use arrow / Ctrl-j/k for move so typing works.
                if c == 'j' && self.search.query.is_empty() {
                    self.search.move_by(1);
                } else if c == 'k' && self.search.query.is_empty() {
                    self.search.move_by(-1);
                } else {
                    self.search.query.push(c);
                    self.search.refresh(&self.home.recent, &self.cfg.cache_dir);
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_stats_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Char('q') => Ok(true),
            KeyCode::Esc => {
                self.stats = None;
                self.screen = Screen::Home;
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_tree_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Char('q') => Ok(true),
            KeyCode::Esc => {
                self.return_to_home()?;
                Ok(false)
            }
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
            KeyCode::Char(' ') => {
                if let Some(s) = &mut self.session {
                    let progress = s.progress.clone();
                    s.tree.toggle_collapse(&progress);
                }
                Ok(false)
            }
            KeyCode::Enter => {
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

    fn handle_typing_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        let cmd = match key.code {
            KeyCode::Esc => TypingCommand::Escape,
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
            if state == SessionState::Completed || state == SessionState::Interrupted {
                Some(state == SessionState::Completed)
            } else {
                None
            }
        });
        if let Some(completed) = finished {
            self.finish_typing(completed)?;
        }
        Ok(false)
    }

    fn handle_result_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Char('q') => Ok(true),
            KeyCode::Enter => {
                self.return_to_home()?;
                Ok(false)
            }
            KeyCode::Esc | KeyCode::Char('r') | KeyCode::Char('t') => {
                if let Some(s) = &mut self.session {
                    s.result = None;
                    s.tree.refresh_rows(&s.progress);
                }
                self.screen = Screen::Tree;
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn open_from_home(&mut self, input: &str) -> crate::Result<()> {
        match load_session(input, &self.cfg, &mut self.store) {
            Ok(session) => {
                self.session = Some(session);
                self.home.error = None;
                self.screen = Screen::Tree;
                self.splash_started = None;
                Ok(())
            }
            Err(err) => {
                self.home.error = Some(err.to_string());
                self.screen = Screen::Home;
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
        self.reload_home_data()?;
        self.screen = Screen::Home;
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
        let label = format!("lines {}–{}", cp.chunk.start_line, cp.chunk.end_line);

        let started_ms = now_millis();
        session.engine = Some(TypingEngine::new(&normalized, started_ms, true, 4));
        self.fx = FxState::new();
        session.typing_path = path.to_string();
        session.typing_chunk_label = label;
        session.typing_chunk_id = chunk_id;
        session.session_started_at = Utc::now().to_rfc3339();
        session.result = None;
        self.screen = Screen::Typing;

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

        session.result = Some(ResultView {
            path: typing_path,
            completed,
            metrics,
            file_done,
        });
        session.tree.refresh_rows(&session.progress);
        self.store.record_session(&summary)?;
        self.screen = Screen::Result;
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

    let (progress_repo, scan, single_file) = if resolved.root.is_dir() {
        if resolved.identity.starts_with("local:") {
            let local_path = resolved.identity.trim_start_matches("local:");
            let p = PathBuf::from(local_path);
            if p.is_file() {
                let (parent, scan) = single_file_scan(&p, WalkOptions::default())?;
                let name = p
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let mut resolved_root = resolved.clone();
                resolved_root.root = parent;
                let (repo_id, progress) = store.sync_scan(&resolved_root, &scan)?;
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
                ));
            }
        }
        let scan = scan_repository(&resolved.root, WalkOptions::default())?;
        (resolved, scan, None)
    } else {
        return Err(Error::InvalidPath(resolved.root));
    };

    if !scan.has_typeable_content() {
        return Err(Error::NoChunks);
    }
    let (repo_id, progress) = store.sync_scan(&progress_repo, &scan)?;
    Ok(RepoSession::from_parts(
        progress_repo,
        repo_id,
        progress,
        single_file,
    ))
}

impl RepoSession {
    fn from_parts(
        repo: ResolvedRepository,
        repo_id: i64,
        progress: RepoProgress,
        single_file: Option<String>,
    ) -> Self {
        let tree = TreeView::from_progress(&repo.display_name, &progress);
        Self {
            repo,
            repo_id,
            progress,
            tree,
            engine: None,
            typing_path: String::new(),
            typing_chunk_label: String::new(),
            typing_chunk_id: 0,
            session_started_at: String::new(),
            result: None,
            single_file,
        }
    }
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
        App::open(
            path,
            &AppConfig {
                cache_dir: cache.to_path_buf(),
                db_path: db.to_path_buf(),
                refresh: false,
                fx_enabled: false,
            },
        )
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
    use tempfile::tempdir;

    #[test]
    fn home_stays_until_open() {
        let dir = tempdir().unwrap();
        let cfg = AppConfig {
            cache_dir: dir.path().join("cache"),
            db_path: dir.path().join("db.sqlite"),
            refresh: false,
            fx_enabled: false,
        };
        let app = App::home(&cfg).unwrap();
        assert_eq!(app.screen, Screen::Home);
        assert!(app.session.is_none());
    }
}
