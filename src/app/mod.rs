//! Application state machine: Tree → Typing → Result.

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
use crate::ui::result::{draw_result, ResultView};
use crate::ui::terminal::TerminalGuard;
use crate::ui::tree::{draw_tree, TreeView};
use crate::ui::typing::draw_typing;
use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Tree,
    Typing,
    Result,
}

pub struct AppConfig {
    pub cache_dir: PathBuf,
    pub db_path: PathBuf,
    pub refresh: bool,
}

pub struct App {
    #[allow(dead_code)]
    repo: ResolvedRepository,
    repo_id: i64,
    progress: RepoProgress,
    store: SqliteStore,
    tree: TreeView,
    screen: Screen,
    engine: Option<TypingEngine>,
    typing_path: String,
    typing_chunk_label: String,
    typing_chunk_id: i64,
    session_started_at: String,
    result: Option<ResultView>,
    /// When opening a single file, only that relative path is the focus root display.
    single_file: Option<String>,
}

impl App {
    pub fn open(input: &str, cfg: &AppConfig) -> crate::Result<Self> {
        let resolved = resolve_source(input, &cfg.cache_dir, cfg.refresh)?;
        let mut store = SqliteStore::open(&cfg.db_path)?;

        let (root, scan, single_file) = if resolved.root.is_dir() {
            // If identity points at a file path (local file open), scan only that file.
            if resolved.identity.starts_with("local:") {
                let local_path = resolved.identity.trim_start_matches("local:");
                let p = PathBuf::from(local_path);
                if p.is_file() {
                    let (parent, scan) = single_file_scan(&p, WalkOptions::default())?;
                    let name = p
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    // Ensure root matches parent used for relative paths.
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
                    return Ok(Self::from_parts(
                        resolved_root,
                        repo_id,
                        progress,
                        store,
                        Some(name),
                    ));
                }
            }
            let scan = scan_repository(&resolved.root, WalkOptions::default())?;
            (resolved.root.clone(), scan, None)
        } else {
            return Err(Error::InvalidPath(resolved.root));
        };

        let _ = root;
        if !scan.has_typeable_content() {
            return Err(Error::NoChunks);
        }
        let (repo_id, progress) = store.sync_scan(&resolved, &scan)?;
        Ok(Self::from_parts(
            resolved,
            repo_id,
            progress,
            store,
            single_file,
        ))
    }

    fn from_parts(
        repo: ResolvedRepository,
        repo_id: i64,
        progress: RepoProgress,
        store: SqliteStore,
        single_file: Option<String>,
    ) -> Self {
        let tree = TreeView::from_progress(&repo.display_name, &progress);
        Self {
            repo,
            repo_id,
            progress,
            store,
            tree,
            screen: Screen::Tree,
            engine: None,
            typing_path: String::new(),
            typing_chunk_label: String::new(),
            typing_chunk_id: 0,
            session_started_at: String::new(),
            result: None,
            single_file,
        }
    }

    pub fn progress(&self) -> &RepoProgress {
        &self.progress
    }

    pub fn run(&mut self) -> crate::Result<()> {
        let mut guard = TerminalGuard::enter()?;
        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(16);

        loop {
            let now_ms = now_millis();
            {
                let term = guard.terminal();
                term.draw(|frame| {
                    let area = frame.area();
                    match self.screen {
                        Screen::Tree => draw_tree(frame, area, &self.tree),
                        Screen::Typing => {
                            if let Some(engine) = &self.engine {
                                draw_typing(
                                    frame,
                                    area,
                                    &self.typing_path,
                                    &self.typing_chunk_label,
                                    &engine.snapshot(),
                                    now_ms,
                                );
                            }
                        }
                        Screen::Result => {
                            if let Some(view) = &self.result {
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
                if let Some(engine) = &mut self.engine {
                    engine.apply(TypingCommand::Tick {
                        now_ms: now_millis(),
                    });
                    let state = engine.snapshot().state;
                    if state == SessionState::Completed || state == SessionState::Interrupted {
                        self.finish_typing(state == SessionState::Completed)?;
                    }
                }
                last_tick = Instant::now();
            }
        }

        guard.restore()?;
        let _ = self.store.touch_repository(self.repo_id);
        Ok(())
    }

    /// Returns true when the app should quit.
    fn handle_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Ok(true);
        }

        match self.screen {
            Screen::Tree => self.handle_tree_key(key),
            Screen::Typing => self.handle_typing_key(key),
            Screen::Result => self.handle_result_key(key),
        }
    }

    fn handle_tree_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Char('q') => Ok(true),
            KeyCode::Esc => Ok(true),
            KeyCode::Char('j') | KeyCode::Down => {
                self.tree.move_by(1);
                Ok(false)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.tree.move_by(-1);
                Ok(false)
            }
            KeyCode::Char(' ') => {
                self.tree.toggle_collapse(&self.progress);
                Ok(false)
            }
            KeyCode::Enter => {
                if let Some(path) = self.tree.selected_file_path() {
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
        if let Some(engine) = &mut self.engine {
            engine.apply(TypingCommand::Tick {
                now_ms: now_millis(),
            });
            engine.apply(cmd);
            let state = engine.snapshot().state;
            if state == SessionState::Completed || state == SessionState::Interrupted {
                self.finish_typing(state == SessionState::Completed)?;
            }
        }
        Ok(false)
    }

    fn handle_result_key(&mut self, key: KeyEvent) -> crate::Result<bool> {
        match key.code {
            KeyCode::Char('q') => Ok(true),
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('r') => {
                self.result = None;
                self.screen = Screen::Tree;
                self.tree.refresh_rows(&self.progress);
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn open_file(&mut self, path: &str) -> crate::Result<()> {
        let file = self
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
        // File-unit typing: always open the sole body (re-challenge allowed).
        self.start_file(path, 0)?;
        Ok(())
    }

    fn start_file(&mut self, path: &str, idx: usize) -> crate::Result<()> {
        let file = self
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

        let started_ms = now_millis();
        self.engine = Some(TypingEngine::new(&cp.chunk.normalized, started_ms, true, 4));
        self.typing_path = path.to_string();
        self.typing_chunk_label = format!("lines {}–{}", cp.chunk.start_line, cp.chunk.end_line);
        self.typing_chunk_id = chunk_id;
        self.session_started_at = Utc::now().to_rfc3339();
        self.result = None;
        self.screen = Screen::Typing;

        // Empty / already auto-completed edge case.
        if let Some(engine) = &self.engine {
            if engine.snapshot().state == SessionState::Completed {
                self.finish_typing(true)?;
            }
        }
        let _ = self.single_file;
        Ok(())
    }

    fn finish_typing(&mut self, completed: bool) -> crate::Result<()> {
        let Some(engine) = self.engine.take() else {
            return Ok(());
        };
        let snap = engine.snapshot();
        let metrics = TypingMetrics::from_counts(snap.keystrokes, snap.misses, snap.elapsed_ms);
        let ended = Utc::now().to_rfc3339();
        let summary = SessionSummary {
            chunk_id: self.typing_chunk_id,
            started_at: self.session_started_at.clone(),
            ended_at: ended,
            completed,
            keystrokes: snap.keystrokes,
            misses: snap.misses,
            elapsed_ms: snap.elapsed_ms,
        };
        self.store.record_session(&summary)?;

        // Update in-memory progress.
        if completed {
            if let Some(file) = self
                .progress
                .files
                .iter_mut()
                .find(|f| f.relative_path == self.typing_path)
            {
                if let Some(c) = file
                    .chunks
                    .iter_mut()
                    .find(|c| c.id == Some(self.typing_chunk_id))
                {
                    c.completion = ChunkCompletion::Complete;
                }
                file.status = file.derive_status();
            }
        }

        let file_done = self
            .progress
            .files
            .iter()
            .find(|f| f.relative_path == self.typing_path)
            .map(|f| f.derive_status() == FileStatus::Done)
            .unwrap_or(false);

        self.result = Some(ResultView {
            path: self.typing_path.clone(),
            completed,
            metrics,
            file_done,
        });
        self.screen = Screen::Result;
        self.tree.refresh_rows(&self.progress);
        Ok(())
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
            },
        )
    }

    pub fn complete_recommended(app: &mut App) -> crate::Result<TypingMetrics> {
        let path = app
            .progress
            .recommend_path()
            .ok_or_else(|| Error::NoChunks)?
            .to_string();
        app.start_file(&path, 0)?;
        // `start_file` may already finish empty/auto-completed bodies.
        if app.engine.is_none() {
            return Ok(app
                .result
                .as_ref()
                .map(|r| r.metrics.clone())
                .unwrap_or_else(|| TypingMetrics::from_counts(0, 0, 0)));
        }
        if let Some(engine) = &mut app.engine {
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
        let completed = app
            .engine
            .as_ref()
            .map(|e| e.snapshot().state == SessionState::Completed)
            .unwrap_or(true);
        app.finish_typing(completed)?;
        Ok(app
            .result
            .as_ref()
            .map(|r| r.metrics.clone())
            .unwrap_or_else(|| TypingMetrics::from_counts(0, 0, 0)))
    }
}
