//! SQLite persistence for repositories, chunks, and sessions.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::{NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::domain::content::{
    ChunkCompletion, ChunkProgress, FileProgress, FileStatus, ManualOverride, RepoProgress,
    ResolvedRepository, ScanResult, SessionSummary, SkipReason, TypingCheckpoint,
};
use crate::domain::file_type::{FileTypePrefs, FileTypeState};
use crate::Error;

/// A previously opened repository for the home Recent list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentRepo {
    pub id: i64,
    pub identity: String,
    pub display_name: String,
    pub input: String,
    pub root_path: String,
    pub last_opened_at: String,
    pub done_lines: usize,
    pub total_lines: usize,
}

/// Aggregate achievement stats across all repositories.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GlobalSummary {
    pub completed_files: usize,
    pub completed_chunks: usize,
    pub streak_days: u32,
}

const SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS repositories (
    id INTEGER PRIMARY KEY NOT NULL,
    identity TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    input TEXT NOT NULL,
    root_path TEXT NOT NULL,
    last_opened_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS repo_files (
    id INTEGER PRIMARY KEY NOT NULL,
    repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    skip_reason TEXT,
    UNIQUE(repository_id, relative_path)
);

CREATE TABLE IF NOT EXISTS chunks (
    id INTEGER PRIMARY KEY NOT NULL,
    file_id INTEGER NOT NULL REFERENCES repo_files(id) ON DELETE CASCADE,
    content_hash TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    normalized TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS chunk_progress (
    chunk_id INTEGER PRIMARY KEY NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    completed INTEGER NOT NULL DEFAULT 0,
    completed_at TEXT
);

CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY NOT NULL,
    chunk_id INTEGER NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL,
    ended_at TEXT NOT NULL,
    completed INTEGER NOT NULL,
    keystrokes INTEGER NOT NULL,
    misses INTEGER NOT NULL,
    elapsed_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS typing_checkpoints (
    chunk_id INTEGER PRIMARY KEY NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    cursor INTEGER NOT NULL,
    keystrokes INTEGER NOT NULL,
    misses INTEGER NOT NULL,
    elapsed_ms INTEGER NOT NULL,
    started_at TEXT NOT NULL,
    auto_indent INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS repo_file_types (
    repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    type_key TEXT NOT NULL,
    state TEXT NOT NULL,
    PRIMARY KEY (repository_id, type_key)
);

CREATE INDEX IF NOT EXISTS idx_chunks_hash ON chunks(content_hash);
CREATE INDEX IF NOT EXISTS idx_chunks_file ON chunks(file_id);
"#;

/// Progress store backed by SQLite.
pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    pub fn open(path: &Path) -> crate::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> crate::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> crate::Result<()> {
        // Pre-release schema churn: `repo_file_types` briefly shipped with an
        // `enabled INTEGER` column before switching to `state TEXT`. Drop and
        // let `CREATE TABLE IF NOT EXISTS` below recreate it; the toggles it
        // held are cheap to lose (the overlay just reappears on next open).
        drop_table_if_missing_column(&self.conn, "repo_file_types", "state")?;

        self.conn.execute_batch(SCHEMA)?;
        self.conn.execute(
            r#"
            INSERT INTO meta(key, value) VALUES ('schema_version', '2')
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            "#,
            [],
        )?;
        ensure_column(&self.conn, "repo_files", "manual_override", "TEXT")?;
        Ok(())
    }

    /// Upsert repository row and merge scan with existing chunk completions by hash.
    ///
    /// When `keep_done_on_refresh` is false, previously completed hashes are not
    /// carried over even if content matches.
    pub fn sync_scan(
        &mut self,
        repo: &ResolvedRepository,
        scan: &ScanResult,
        keep_done_on_refresh: bool,
    ) -> crate::Result<(i64, RepoProgress)> {
        let tx = self.conn.transaction()?;
        let now = Utc::now().to_rfc3339();
        let repo_id = upsert_repository(&tx, repo, &now)?;

        // Existing completions: hash -> completed
        let mut completed_hashes = std::collections::HashSet::new();
        if keep_done_on_refresh {
            let mut stmt = tx.prepare(
                r#"
                SELECT c.content_hash
                FROM chunks c
                JOIN repo_files f ON f.id = c.file_id
                JOIN chunk_progress p ON p.chunk_id = c.id
                WHERE f.repository_id = ?1 AND p.completed = 1
                "#,
            )?;
            let rows = stmt.query_map(params![repo_id], |row| row.get::<_, String>(0))?;
            for h in rows {
                completed_hashes.insert(h?);
            }
        }

        // Mark all existing chunks inactive; re-activate / insert from scan.
        tx.execute(
            r#"
            UPDATE chunks SET active = 0
            WHERE file_id IN (SELECT id FROM repo_files WHERE repository_id = ?1)
            "#,
            params![repo_id],
        )?;

        for scanned in &scan.files {
            let skip = scanned
                .skip_reason
                .as_ref()
                .map(|r| r.as_str())
                .unwrap_or_default();
            let skip_param: Option<&str> = if scanned.status == FileStatus::Skipped {
                Some(if skip.is_empty() { "skipped" } else { &skip })
            } else {
                None
            };

            tx.execute(
                r#"
                INSERT INTO repo_files(repository_id, relative_path, skip_reason)
                VALUES (?1, ?2, ?3)
                ON CONFLICT(repository_id, relative_path) DO UPDATE SET
                    skip_reason = excluded.skip_reason
                "#,
                params![repo_id, scanned.relative_path, skip_param],
            )?;
            let file_id: i64 = tx.query_row(
                "SELECT id FROM repo_files WHERE repository_id = ?1 AND relative_path = ?2",
                params![repo_id, scanned.relative_path],
                |row| row.get(0),
            )?;

            for chunk in &scanned.chunks {
                // Reuse row with same hash on this file if present.
                let existing: Option<i64> = tx
                    .query_row(
                        "SELECT id FROM chunks WHERE file_id = ?1 AND content_hash = ?2",
                        params![file_id, chunk.hash],
                        |row| row.get(0),
                    )
                    .optional()?;

                let chunk_id = if let Some(id) = existing {
                    tx.execute(
                        r#"
                        UPDATE chunks
                        SET start_line = ?1, end_line = ?2, normalized = ?3, active = 1
                        WHERE id = ?4
                        "#,
                        params![chunk.start_line, chunk.end_line, chunk.normalized, id],
                    )?;
                    id
                } else {
                    tx.execute(
                        r#"
                        INSERT INTO chunks(file_id, content_hash, start_line, end_line, normalized, active)
                        VALUES (?1, ?2, ?3, ?4, ?5, 1)
                        "#,
                        params![
                            file_id,
                            chunk.hash,
                            chunk.start_line,
                            chunk.end_line,
                            chunk.normalized
                        ],
                    )?;
                    tx.last_insert_rowid()
                };

                tx.execute(
                    r#"
                    INSERT INTO chunk_progress(chunk_id, completed, completed_at)
                    VALUES (?1, 0, NULL)
                    ON CONFLICT(chunk_id) DO NOTHING
                    "#,
                    params![chunk_id],
                )?;

                let was_done = completed_hashes.contains(&chunk.hash);
                if was_done {
                    tx.execute(
                        r#"
                        UPDATE chunk_progress
                        SET completed = 1,
                            completed_at = COALESCE(completed_at, ?2)
                        WHERE chunk_id = ?1
                        "#,
                        params![chunk_id, now],
                    )?;
                }
            }
        }

        // A changed or removed normalized body must not retain its old cursor.
        tx.execute(
            "DELETE FROM typing_checkpoints WHERE chunk_id IN (SELECT id FROM chunks WHERE active = 0)",
            [],
        )?;

        tx.commit()?;
        Ok((repo_id, self.load_progress(repo_id)?))
    }

    pub fn load_progress(&self, repo_id: i64) -> crate::Result<RepoProgress> {
        let mut files = Vec::new();
        let mut file_stmt = self.conn.prepare(
            "SELECT id, relative_path, skip_reason, manual_override FROM repo_files WHERE repository_id = ?1 ORDER BY relative_path",
        )?;
        let file_rows: Vec<(i64, String, Option<String>, Option<String>)> = file_stmt
            .query_map(params![repo_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        for (file_id, relative_path, skip_reason, manual_override) in file_rows {
            let mut chunk_stmt = self.conn.prepare(
                r#"
                SELECT c.id, c.content_hash, c.start_line, c.end_line, c.normalized,
                       COALESCE(p.completed, 0),
                       tc.cursor, tc.keystrokes, tc.misses, tc.elapsed_ms,
                       tc.started_at, tc.auto_indent
                FROM chunks c
                LEFT JOIN chunk_progress p ON p.chunk_id = c.id
                LEFT JOIN typing_checkpoints tc ON tc.chunk_id = c.id
                WHERE c.file_id = ?1 AND c.active = 1
                ORDER BY c.start_line, c.id
                "#,
            )?;
            let chunks = chunk_stmt
                .query_map(params![file_id], |row| {
                    let id: i64 = row.get(0)?;
                    let hash: String = row.get(1)?;
                    let start_line: u32 = row.get(2)?;
                    let end_line: u32 = row.get(3)?;
                    let normalized: String = row.get(4)?;
                    let completed: i64 = row.get(5)?;
                    let cursor: Option<i64> = row.get(6)?;
                    let checkpoint = if let Some(cursor) = cursor {
                        Some(TypingCheckpoint {
                            chunk_id: id,
                            cursor: cursor.max(0) as usize,
                            keystrokes: row
                                .get::<_, Option<i64>>(7)?
                                .unwrap_or(0)
                                .clamp(0, i64::from(u32::MAX))
                                as u32,
                            misses: row
                                .get::<_, Option<i64>>(8)?
                                .unwrap_or(0)
                                .clamp(0, i64::from(u32::MAX))
                                as u32,
                            elapsed_ms: row.get::<_, Option<i64>>(9)?.unwrap_or(0).max(0) as u64,
                            started_at: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
                            auto_indent: row.get::<_, Option<i64>>(11)?.unwrap_or(0) != 0,
                        })
                    } else {
                        None
                    };
                    Ok(ChunkProgress {
                        chunk: crate::domain::content::Chunk {
                            relative_path: relative_path.clone(),
                            start_line,
                            end_line,
                            normalized,
                            hash,
                        },
                        completion: if completed == 1 {
                            ChunkCompletion::Complete
                        } else {
                            ChunkCompletion::Incomplete
                        },
                        checkpoint,
                        id: Some(id),
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            let skip_reason = skip_reason.as_deref().map(parse_skip_reason);

            let status = if skip_reason.is_some() || chunks.is_empty() {
                FileStatus::Skipped
            } else {
                FileStatus::Todo
            };

            let mut fp = FileProgress {
                relative_path,
                status,
                skip_reason,
                manual_override: manual_override.as_deref().and_then(ManualOverride::parse),
                chunks,
            };
            fp.status = fp.derive_status();
            files.push(fp);
        }

        Ok(RepoProgress { files })
    }

    /// Record a finalized session; mark chunk complete and clear its checkpoint when finished.
    pub fn record_session(&mut self, summary: &SessionSummary) -> crate::Result<()> {
        let tx = self.conn.transaction()?;
        save_session(&tx, summary)?;
        if summary.completed {
            tx.execute(
                r#"
                UPDATE chunk_progress
                SET completed = 1, completed_at = ?2
                WHERE chunk_id = ?1
                "#,
                params![summary.chunk_id, summary.ended_at],
            )?;
        }
        tx.execute(
            "DELETE FROM typing_checkpoints WHERE chunk_id = ?1",
            params![summary.chunk_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Upsert the latest suspended typing state for a chunk.
    pub fn save_checkpoint(&mut self, checkpoint: &TypingCheckpoint) -> crate::Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            r#"
            INSERT INTO typing_checkpoints(
                chunk_id, cursor, keystrokes, misses, elapsed_ms, started_at, auto_indent, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(chunk_id) DO UPDATE SET
                cursor = excluded.cursor,
                keystrokes = excluded.keystrokes,
                misses = excluded.misses,
                elapsed_ms = excluded.elapsed_ms,
                started_at = excluded.started_at,
                auto_indent = excluded.auto_indent,
                updated_at = excluded.updated_at
            "#,
            params![
                checkpoint.chunk_id,
                checkpoint.cursor as i64,
                checkpoint.keystrokes,
                checkpoint.misses,
                checkpoint.elapsed_ms as i64,
                &checkpoint.started_at,
                if checkpoint.auto_indent { 1 } else { 0 },
                now,
            ],
        )?;
        Ok(())
    }

    pub fn clear_checkpoint(&mut self, chunk_id: i64) -> crate::Result<()> {
        self.conn.execute(
            "DELETE FROM typing_checkpoints WHERE chunk_id = ?1",
            params![chunk_id],
        )?;
        Ok(())
    }

    /// Persist a manual skip/include override. `None` restores automatic detection.
    pub fn set_manual_override(
        &self,
        repo_id: i64,
        relative_path: &str,
        override_value: Option<ManualOverride>,
    ) -> crate::Result<()> {
        let value = override_value.map(ManualOverride::as_str);
        let changed = self.conn.execute(
            r#"
            UPDATE repo_files
            SET manual_override = ?3
            WHERE repository_id = ?1 AND relative_path = ?2
            "#,
            params![repo_id, relative_path, value],
        )?;
        if changed == 0 {
            return Err(Error::Message(format!("file not found: {relative_path}")));
        }
        Ok(())
    }

    /// Repository id for an already-known identity, if it has been opened before.
    pub fn find_repository_id(&self, identity: &str) -> crate::Result<Option<i64>> {
        let id = self
            .conn
            .query_row(
                "SELECT id FROM repositories WHERE identity = ?1",
                params![identity],
                |row| row.get(0),
            )
            .optional()?;
        Ok(id)
    }

    /// Saved file-type toggles for a repository (empty if never configured).
    pub fn load_file_type_prefs(&self, repo_id: i64) -> crate::Result<FileTypePrefs> {
        let mut stmt = self
            .conn
            .prepare("SELECT type_key, state FROM repo_file_types WHERE repository_id = ?1")?;
        let rows: std::collections::HashMap<String, FileTypeState> = stmt
            .query_map(params![repo_id], |row| {
                let key: String = row.get(0)?;
                let state: String = row.get(1)?;
                Ok((key, state))
            })?
            .collect::<std::result::Result<Vec<(String, String)>, _>>()?
            .into_iter()
            .filter_map(|(key, state)| FileTypeState::parse(&state).map(|s| (key, s)))
            .collect();
        Ok(FileTypePrefs::new(rows))
    }

    /// Replace all saved file-type toggles for a repository in one transaction.
    pub fn save_file_type_prefs(
        &mut self,
        repo_id: i64,
        prefs: &[(String, FileTypeState)],
    ) -> crate::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM repo_file_types WHERE repository_id = ?1",
            params![repo_id],
        )?;
        for (key, state) in prefs {
            tx.execute(
                "INSERT INTO repo_file_types(repository_id, type_key, state) VALUES (?1, ?2, ?3)",
                params![repo_id, key, state.as_str()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn touch_repository(&self, repo_id: i64) -> crate::Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE repositories SET last_opened_at = ?1 WHERE id = ?2",
            params![now, repo_id],
        )?;
        Ok(())
    }

    /// Repositories ordered by most recently opened, with line progress attached.
    pub fn list_recent_repos(&self, limit: usize) -> crate::Result<Vec<RecentRepo>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, identity, display_name, input, root_path, last_opened_at
            FROM repositories
            ORDER BY last_opened_at DESC
            LIMIT ?1
            "#,
        )?;
        let rows: Vec<(i64, String, String, String, String, String)> = stmt
            .query_map(params![limit as i64], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut out = Vec::with_capacity(rows.len());
        for (id, identity, display_name, input, root_path, last_opened_at) in rows {
            let (done_lines, total_lines) = self.repo_line_progress(id)?;
            out.push(RecentRepo {
                id,
                identity,
                display_name,
                input,
                root_path,
                last_opened_at,
                done_lines,
                total_lines,
            });
        }
        Ok(out)
    }

    /// `(completed_lines, total_lines)` for non-skipped files in a repository.
    pub fn repo_line_progress(&self, repo_id: i64) -> crate::Result<(usize, usize)> {
        let progress = self.load_progress(repo_id)?;
        Ok((progress.completed_lines(), progress.total_lines()))
    }

    /// Completed files / chunks across all repos, plus consecutive activity days.
    pub fn global_summary(&self) -> crate::Result<GlobalSummary> {
        let completed_chunks: i64 = self.conn.query_row(
            r#"
            SELECT COUNT(*)
            FROM chunk_progress p
            JOIN chunks c ON c.id = p.chunk_id
            WHERE p.completed = 1 AND c.active = 1
            "#,
            [],
            |row| row.get(0),
        )?;

        // A file is complete when it has ≥1 active chunk and all are completed,
        // and it is not skipped.
        let completed_files: i64 = self.conn.query_row(
            r#"
            SELECT COUNT(*) FROM (
                SELECT f.id
                FROM repo_files f
                JOIN chunks c ON c.file_id = f.id AND c.active = 1
                LEFT JOIN chunk_progress p ON p.chunk_id = c.id
                WHERE COALESCE(f.manual_override, '') != 'skip'
                  AND (
                    f.manual_override = 'include'
                    OR (f.manual_override IS NULL AND f.skip_reason IS NULL)
                  )
                GROUP BY f.id
                HAVING COUNT(c.id) > 0
                   AND SUM(CASE WHEN COALESCE(p.completed, 0) = 1 THEN 1 ELSE 0 END) = COUNT(c.id)
            )
            "#,
            [],
            |row| row.get(0),
        )?;

        let mut dates = BTreeSet::new();
        {
            let mut stmt = self.conn.prepare(
                r#"
                SELECT ended_at FROM sessions WHERE completed = 1
                "#,
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            for ended in rows {
                let ended = ended?;
                if let Some(day) = parse_session_day(&ended) {
                    dates.insert(day);
                }
            }
        }

        Ok(GlobalSummary {
            completed_files: completed_files as usize,
            completed_chunks: completed_chunks as usize,
            streak_days: compute_streak(&dates, Utc::now().date_naive()),
        })
    }
}

/// Parse an RFC3339 (or date-prefixed) timestamp into a calendar day.
fn parse_session_day(ended_at: &str) -> Option<NaiveDate> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ended_at) {
        return Some(dt.date_naive());
    }
    // Fallback: leading YYYY-MM-DD.
    if ended_at.len() >= 10 {
        return NaiveDate::parse_from_str(&ended_at[..10], "%Y-%m-%d").ok();
    }
    None
}

/// Consecutive days with activity ending on `today` or `yesterday`.
fn compute_streak(dates: &BTreeSet<NaiveDate>, today: NaiveDate) -> u32 {
    if dates.is_empty() {
        return 0;
    }
    let yesterday = today.pred_opt().unwrap_or(today);
    let start = if dates.contains(&today) {
        today
    } else if dates.contains(&yesterday) {
        yesterday
    } else {
        return 0;
    };
    let mut streak = 0u32;
    let mut cursor = start;
    while dates.contains(&cursor) {
        streak += 1;
        match cursor.pred_opt() {
            Some(prev) => cursor = prev,
            None => break,
        }
    }
    streak
}

fn upsert_repository(
    tx: &Transaction<'_>,
    repo: &ResolvedRepository,
    now: &str,
) -> crate::Result<i64> {
    let kind = match repo.kind {
        crate::domain::content::SourceKind::Local => "local",
        crate::domain::content::SourceKind::GitHub => "github",
    };
    tx.execute(
        r#"
        INSERT INTO repositories(identity, display_name, source_kind, input, root_path, last_opened_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(identity) DO UPDATE SET
            display_name = excluded.display_name,
            source_kind = excluded.source_kind,
            input = excluded.input,
            root_path = excluded.root_path,
            last_opened_at = excluded.last_opened_at
        "#,
        params![
            repo.identity,
            repo.display_name,
            kind,
            repo.input,
            repo.root.to_string_lossy().as_ref(),
            now
        ],
    )?;
    let id = tx.query_row(
        "SELECT id FROM repositories WHERE identity = ?1",
        params![repo.identity],
        |row| row.get(0),
    )?;
    Ok(id)
}

fn save_session(tx: &Transaction<'_>, summary: &SessionSummary) -> crate::Result<()> {
    tx.execute(
        r#"
        INSERT INTO sessions(chunk_id, started_at, ended_at, completed, keystrokes, misses, elapsed_ms)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
        params![
            summary.chunk_id,
            summary.started_at,
            summary.ended_at,
            if summary.completed { 1 } else { 0 },
            summary.keystrokes,
            summary.misses,
            summary.elapsed_ms as i64,
        ],
    )?;
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> crate::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(std::result::Result::ok)
        .any(|name| name == column);
    if !exists {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"),
            [],
        )?;
    }
    Ok(())
}

/// Drop `table` if it exists but predates `column` (an incompatible earlier shape).
/// A no-op when the table doesn't exist yet or already has the column.
fn drop_table_if_missing_column(conn: &Connection, table: &str, column: &str) -> crate::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<_, _>>()?;
    if !columns.is_empty() && !columns.iter().any(|name| name == column) {
        conn.execute(&format!("DROP TABLE {table}"), [])?;
    }
    Ok(())
}

fn parse_skip_reason(raw: &str) -> SkipReason {
    if raw == "excluded directory" {
        SkipReason::VcsOrDependencyDir
    } else if raw == "generated or lock file" {
        SkipReason::GeneratedOrLockFile
    } else if raw == "binary file" {
        SkipReason::Binary
    } else if raw == "test file (include_tests=false)" {
        SkipReason::TestFile
    } else if raw == "config file (include_configs=false)"
        || raw == "config or docs (include_configs=false)"
    {
        SkipReason::ConfigFile
    } else if raw == "file type disabled" {
        SkipReason::FileTypeDisabled
    } else if raw == "no typeable chunks" || raw == "no typeable content" {
        SkipReason::NoChunks
    } else if let Some(rest) = raw.strip_prefix("line longer than ") {
        let max_cols = rest
            .strip_suffix(" cols")
            .and_then(|s| s.parse().ok())
            .unwrap_or(200);
        SkipReason::LineTooLong { max_cols }
    } else if let Some(rest) = raw.strip_prefix("more than ") {
        let max_lines = rest
            .strip_suffix(" lines")
            .and_then(|s| s.parse().ok())
            .unwrap_or(5000);
        SkipReason::FileTooLarge { max_lines }
    } else if let Some(msg) = raw.strip_prefix("read error: ") {
        SkipReason::IoError(msg.to_string())
    } else {
        SkipReason::IoError(raw.to_string())
    }
}

/// Paths managed by repomonk that `--purge` may delete.
#[derive(Debug, Clone)]
pub struct DataPaths {
    pub cache_dir: std::path::PathBuf,
    pub data_dir: std::path::PathBuf,
    pub db_path: std::path::PathBuf,
}

impl DataPaths {
    pub fn from_roots(cache_dir: std::path::PathBuf, data_dir: std::path::PathBuf) -> Self {
        let db_path = data_dir.join("repomonk.db");
        Self {
            cache_dir,
            data_dir,
            db_path,
        }
    }

    pub fn default_user() -> crate::Result<Self> {
        let cache = dirs::cache_dir()
            .ok_or_else(|| Error::Message("cannot resolve cache directory".into()))?
            .join("repomonk");
        let data = dirs::data_local_dir()
            .ok_or_else(|| Error::Message("cannot resolve data directory".into()))?
            .join("repomonk");
        Ok(Self::from_roots(cache, data))
    }
}

/// Delete app-managed cache and data directories without following external symlinks.
pub fn purge(paths: &DataPaths) -> crate::Result<()> {
    purge_tree(&paths.cache_dir)?;
    purge_tree(&paths.data_dir)?;
    Ok(())
}

fn purge_tree(root: &Path) -> crate::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    // If the root itself is a symlink pointing outside, refuse to delete target contents;
    // only remove the symlink.
    if root
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        std::fs::remove_file(root)?;
        return Ok(());
    }
    purge_dir_contents(root, root)?;
    if root.exists() {
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

fn purge_dir_contents(root: &Path, dir: &Path) -> crate::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        // Do not follow symlinks that escape; delete the link itself.
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            // Only remove symlink file, never recurse into it.
            let _ = std::fs::remove_file(&path);
            continue;
        }
        // Safety: path must stay under root.
        let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let canon_path = path.canonicalize().unwrap_or_else(|_| path.clone());
        if !canon_path.starts_with(&canon_root) {
            continue;
        }
        if meta.is_dir() {
            purge_dir_contents(root, &path)?;
            let _ = std::fs::remove_dir(&path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::content::{Chunk, ManualOverride, ScannedFile, SkipReason, SourceKind};
    use crate::scan::extract::hash_normalized;
    use tempfile::tempdir;

    fn sample_chunk(path: &str, body: &str) -> Chunk {
        Chunk {
            relative_path: path.into(),
            start_line: 1,
            end_line: body.lines().count() as u32,
            hash: hash_normalized(body),
            normalized: body.into(),
        }
    }

    #[test]
    fn sync_and_complete_and_restore_by_hash() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let repo = ResolvedRepository {
            identity: "local:/tmp/demo".into(),
            display_name: "demo".into(),
            kind: SourceKind::Local,
            root: "/tmp/demo".into(),
            input: "/tmp/demo".into(),
        };
        let body = "fn main() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n";
        let scan = ScanResult {
            files: vec![ScannedFile {
                relative_path: "src/main.rs".into(),
                status: FileStatus::Todo,
                skip_reason: None,
                chunks: vec![sample_chunk("src/main.rs", body)],
            }],
            import_edges: Vec::new(),
        };
        let (repo_id, progress) = store.sync_scan(&repo, &scan, true).unwrap();
        let chunk_id = progress.files[0].chunks[0].id.unwrap();
        assert_eq!(
            progress.files[0].chunks[0].completion,
            ChunkCompletion::Incomplete
        );

        store
            .record_session(&SessionSummary {
                chunk_id,
                started_at: "t0".into(),
                ended_at: "t1".into(),
                completed: true,
                keystrokes: 10,
                misses: 1,
                elapsed_ms: 1000,
            })
            .unwrap();

        // Re-scan with same body but different line numbers → still complete.
        let body2 = "\n\n".to_string() + body;
        let scan2 = ScanResult {
            files: vec![ScannedFile {
                relative_path: "src/main.rs".into(),
                status: FileStatus::Todo,
                skip_reason: None,
                chunks: vec![Chunk {
                    relative_path: "src/main.rs".into(),
                    start_line: 3,
                    end_line: 7,
                    hash: hash_normalized(body),
                    normalized: body.into(),
                }],
            }],
            import_edges: Vec::new(),
        };
        // Wait — hash of body2 normalized lines differ. Use same normalized hash explicitly.
        let _ = body2;
        let (_, progress2) = store.sync_scan(&repo, &scan2, true).unwrap();
        assert_eq!(
            progress2.files[0].chunks[0].completion,
            ChunkCompletion::Complete
        );

        let loaded = store.load_progress(repo_id).unwrap();
        assert_eq!(loaded.files[0].derive_status(), FileStatus::Done);
    }

    #[test]
    fn checkpoint_round_trips_and_is_cleared_on_completion_or_changed_refresh() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let repo = ResolvedRepository {
            identity: "local:/tmp/checkpoint".into(),
            display_name: "checkpoint".into(),
            kind: SourceKind::Local,
            root: "/tmp/checkpoint".into(),
            input: "/tmp/checkpoint".into(),
        };
        let body = "one\ntwo\nthree";
        let scan = ScanResult {
            files: vec![ScannedFile {
                relative_path: "main.rs".into(),
                status: FileStatus::Todo,
                skip_reason: None,
                chunks: vec![sample_chunk("main.rs", body)],
            }],
            import_edges: Vec::new(),
        };
        let (repo_id, progress) = store.sync_scan(&repo, &scan, true).unwrap();
        let chunk_id = progress.files[0].chunks[0].id.unwrap();
        store
            .save_checkpoint(&TypingCheckpoint {
                chunk_id,
                cursor: 4,
                keystrokes: 4,
                misses: 1,
                elapsed_ms: 2_000,
                started_at: "t0".into(),
                auto_indent: true,
            })
            .unwrap();

        let loaded = store.load_progress(repo_id).unwrap();
        let checkpoint = loaded.files[0].chunks[0]
            .checkpoint
            .as_ref()
            .expect("checkpoint persisted");
        assert_eq!(checkpoint.cursor, 4);
        assert_eq!(loaded.completed_lines(), 1);
        let recent = store.list_recent_repos(1).unwrap();
        assert_eq!(recent[0].done_lines, 1);

        let (_, same_body) = store.sync_scan(&repo, &scan, true).unwrap();
        assert!(same_body.files[0].chunks[0].checkpoint.is_some());

        let changed_scan = ScanResult {
            files: vec![ScannedFile {
                relative_path: "main.rs".into(),
                status: FileStatus::Todo,
                skip_reason: None,
                chunks: vec![sample_chunk("main.rs", "changed")],
            }],
            import_edges: Vec::new(),
        };
        let (_, changed) = store.sync_scan(&repo, &changed_scan, true).unwrap();
        assert!(changed.files[0].chunks[0].checkpoint.is_none());

        let changed_id = changed.files[0].chunks[0].id.unwrap();
        store
            .save_checkpoint(&TypingCheckpoint {
                chunk_id: changed_id,
                cursor: 1,
                keystrokes: 1,
                misses: 0,
                elapsed_ms: 1,
                started_at: "t0".into(),
                auto_indent: false,
            })
            .unwrap();
        store
            .record_session(&SessionSummary {
                chunk_id: changed_id,
                started_at: "t0".into(),
                ended_at: "t1".into(),
                completed: true,
                keystrokes: 7,
                misses: 0,
                elapsed_ms: 100,
            })
            .unwrap();
        assert!(store
            .load_progress(repo_id)
            .unwrap()
            .files
            .iter()
            .flat_map(|file| file.chunks.iter())
            .all(|chunk| chunk.checkpoint.is_none()));
    }

    #[test]
    fn purge_removes_only_managed_trees() {
        let dir = tempdir().unwrap();
        let cache = dir.path().join("cache");
        let data = dir.path().join("data");
        std::fs::create_dir_all(cache.join("repos")).unwrap();
        std::fs::write(cache.join("repos/x"), "1").unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("repomonk.db"), "db").unwrap();
        let paths = DataPaths::from_roots(cache.clone(), data.clone());
        purge(&paths).unwrap();
        assert!(!cache.exists());
        assert!(!data.exists());
    }

    #[test]
    fn list_recent_orders_by_last_opened() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let body = "fn main() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n";
        let scan = ScanResult {
            files: vec![ScannedFile {
                relative_path: "src/main.rs".into(),
                status: FileStatus::Todo,
                skip_reason: None,
                chunks: vec![sample_chunk("src/main.rs", body)],
            }],
            import_edges: Vec::new(),
        };
        let older = ResolvedRepository {
            identity: "local:/tmp/older".into(),
            display_name: "older".into(),
            kind: SourceKind::Local,
            root: "/tmp/older".into(),
            input: "/tmp/older".into(),
        };
        let newer = ResolvedRepository {
            identity: "local:/tmp/newer".into(),
            display_name: "newer".into(),
            kind: SourceKind::Local,
            root: "/tmp/newer".into(),
            input: "/tmp/newer".into(),
        };
        let (older_id, _) = store.sync_scan(&older, &scan, true).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let (_newer_id, _) = store.sync_scan(&newer, &scan, true).unwrap();
        // Touch older so it becomes most recent.
        store.touch_repository(older_id).unwrap();

        let recent = store.list_recent_repos(10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].display_name, "older");
        assert_eq!(recent[1].display_name, "newer");
        assert!(recent[0].total_lines >= 5);
        assert_eq!(recent[0].done_lines, 0);
    }

    #[test]
    fn global_summary_counts_completions_and_streak() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let body = "fn main() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n";
        let repo = ResolvedRepository {
            identity: "local:/tmp/demo".into(),
            display_name: "demo".into(),
            kind: SourceKind::Local,
            root: "/tmp/demo".into(),
            input: "/tmp/demo".into(),
        };
        let scan = ScanResult {
            files: vec![ScannedFile {
                relative_path: "src/main.rs".into(),
                status: FileStatus::Todo,
                skip_reason: None,
                chunks: vec![sample_chunk("src/main.rs", body)],
            }],
            import_edges: Vec::new(),
        };
        let (_, progress) = store.sync_scan(&repo, &scan, true).unwrap();
        let chunk_id = progress.files[0].chunks[0].id.unwrap();
        let today = Utc::now().to_rfc3339();
        store
            .record_session(&SessionSummary {
                chunk_id,
                started_at: today.clone(),
                ended_at: today,
                completed: true,
                keystrokes: 10,
                misses: 0,
                elapsed_ms: 100,
            })
            .unwrap();

        let summary = store.global_summary().unwrap();
        assert_eq!(summary.completed_chunks, 1);
        assert_eq!(summary.completed_files, 1);
        assert!(summary.streak_days >= 1);
    }

    #[test]
    fn compute_streak_allows_yesterday_start() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        let mut dates = BTreeSet::new();
        dates.insert(NaiveDate::from_ymd_opt(2026, 8, 10).unwrap());
        dates.insert(NaiveDate::from_ymd_opt(2026, 8, 9).unwrap());
        assert_eq!(compute_streak(&dates, today), 2);
        assert_eq!(compute_streak(&BTreeSet::new(), today), 0);
    }

    #[test]
    fn manual_override_survives_refresh() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let body = "fn main() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n";
        let repo = ResolvedRepository {
            identity: "local:/tmp/demo".into(),
            display_name: "demo".into(),
            kind: SourceKind::Local,
            root: "/tmp/demo".into(),
            input: "/tmp/demo".into(),
        };
        let scan = ScanResult {
            files: vec![ScannedFile {
                relative_path: "src/main.rs".into(),
                status: FileStatus::Todo,
                skip_reason: None,
                chunks: vec![sample_chunk("src/main.rs", body)],
            }],
            import_edges: Vec::new(),
        };
        let (repo_id, progress) = store.sync_scan(&repo, &scan, true).unwrap();
        assert_eq!(progress.files[0].derive_status(), FileStatus::Todo);

        store
            .set_manual_override(repo_id, "src/main.rs", Some(ManualOverride::Skip))
            .unwrap();
        let loaded = store.load_progress(repo_id).unwrap();
        assert_eq!(loaded.files[0].derive_status(), FileStatus::Skipped);

        let (_, refreshed) = store.sync_scan(&repo, &scan, true).unwrap();
        assert_eq!(
            refreshed.files[0].manual_override,
            Some(ManualOverride::Skip)
        );
        assert_eq!(refreshed.files[0].derive_status(), FileStatus::Skipped);

        store
            .set_manual_override(repo_id, "src/main.rs", None)
            .unwrap();
        let cleared = store.load_progress(repo_id).unwrap();
        assert_eq!(cleared.files[0].manual_override, None);
        assert_eq!(cleared.files[0].derive_status(), FileStatus::Todo);
    }

    #[test]
    fn parses_current_and_legacy_config_skip_reasons() {
        assert_eq!(
            parse_skip_reason("config or docs (include_configs=false)"),
            SkipReason::ConfigFile
        );
        assert_eq!(
            parse_skip_reason("config file (include_configs=false)"),
            SkipReason::ConfigFile
        );
    }

    #[test]
    fn migrates_pre_release_repo_file_types_shape() {
        // Simulate a DB created before `enabled INTEGER` became `state TEXT`.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE repositories (
                id INTEGER PRIMARY KEY NOT NULL,
                identity TEXT NOT NULL UNIQUE,
                display_name TEXT NOT NULL,
                source_kind TEXT NOT NULL,
                input TEXT NOT NULL,
                root_path TEXT NOT NULL,
                last_opened_at TEXT NOT NULL
            );
            CREATE TABLE repo_file_types (
                repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
                type_key TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                PRIMARY KEY (repository_id, type_key)
            );
            "#,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO repositories(id, identity, display_name, source_kind, input, root_path, last_opened_at) VALUES (1, 'local:/tmp/old', 'old', 'local', '/tmp/old', '/tmp/old', '2026-01-01')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO repo_file_types(repository_id, type_key, enabled) VALUES (1, '.rs', 1)",
            [],
        )
        .unwrap();

        let store = SqliteStore { conn };
        store.migrate().unwrap();

        // Old row is gone (dropped table), but querying by the new shape works.
        let prefs = store.load_file_type_prefs(1).unwrap();
        assert!(prefs.is_empty());
        store
            .conn
            .execute(
                "INSERT INTO repo_file_types(repository_id, type_key, state) VALUES (1, '.rs', 'included')",
                [],
            )
            .unwrap();
        assert_eq!(
            store.load_file_type_prefs(1).unwrap().get(".rs"),
            Some(FileTypeState::Included)
        );
    }

    #[test]
    fn file_type_prefs_round_trip_and_survive_refresh_and_cascade() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let body = "fn main() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n";
        let repo = ResolvedRepository {
            identity: "local:/tmp/types".into(),
            display_name: "types".into(),
            kind: SourceKind::Local,
            root: "/tmp/types".into(),
            input: "/tmp/types".into(),
        };
        let scan = ScanResult {
            files: vec![ScannedFile {
                relative_path: "src/main.rs".into(),
                status: FileStatus::Todo,
                skip_reason: None,
                chunks: vec![sample_chunk("src/main.rs", body)],
            }],
            import_edges: Vec::new(),
        };
        assert_eq!(store.find_repository_id(&repo.identity).unwrap(), None);
        let (repo_id, _) = store.sync_scan(&repo, &scan, true).unwrap();
        assert_eq!(
            store.find_repository_id(&repo.identity).unwrap(),
            Some(repo_id)
        );

        assert!(store.load_file_type_prefs(repo_id).unwrap().is_empty());

        store
            .save_file_type_prefs(
                repo_id,
                &[
                    (".rs".to_string(), FileTypeState::Included),
                    (".md".to_string(), FileTypeState::Excluded),
                    (".png".to_string(), FileTypeState::Hidden),
                ],
            )
            .unwrap();
        let prefs = store.load_file_type_prefs(repo_id).unwrap();
        assert_eq!(prefs.get(".rs"), Some(FileTypeState::Included));
        assert_eq!(prefs.get(".md"), Some(FileTypeState::Excluded));
        assert_eq!(prefs.get(".png"), Some(FileTypeState::Hidden));
        assert_eq!(prefs.get(".json"), None);

        // Refresh should not clear saved prefs.
        store.sync_scan(&repo, &scan, true).unwrap();
        let prefs_after_refresh = store.load_file_type_prefs(repo_id).unwrap();
        assert_eq!(
            prefs_after_refresh.get(".rs"),
            Some(FileTypeState::Included)
        );

        // Overwrite replaces the full set.
        store
            .save_file_type_prefs(repo_id, &[(".rs".to_string(), FileTypeState::Excluded)])
            .unwrap();
        let overwritten = store.load_file_type_prefs(repo_id).unwrap();
        assert_eq!(overwritten.get(".rs"), Some(FileTypeState::Excluded));
        assert_eq!(overwritten.get(".md"), None);

        store
            .conn
            .execute("DELETE FROM repositories WHERE id = ?1", params![repo_id])
            .unwrap();
        assert!(store.load_file_type_prefs(repo_id).unwrap().is_empty());
    }
}
