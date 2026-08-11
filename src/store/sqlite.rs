//! SQLite persistence for repositories, chunks, and sessions.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::{NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::domain::content::{
    ChunkCompletion, ChunkProgress, FileProgress, FileStatus, RepoProgress, ResolvedRepository,
    ScanResult, SessionSummary, SkipReason,
};
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
        self.conn.execute_batch(SCHEMA)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', '1')",
            [],
        )?;
        Ok(())
    }

    /// Upsert repository row and merge scan with existing chunk completions by hash.
    pub fn sync_scan(
        &mut self,
        repo: &ResolvedRepository,
        scan: &ScanResult,
    ) -> crate::Result<(i64, RepoProgress)> {
        let tx = self.conn.transaction()?;
        let now = Utc::now().to_rfc3339();
        let repo_id = upsert_repository(&tx, repo, &now)?;

        // Existing completions: hash -> completed
        let mut completed_hashes = std::collections::HashSet::new();
        {
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

        let mut files_out = Vec::new();

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

            let mut chunk_progresses = Vec::new();
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

                chunk_progresses.push(ChunkProgress {
                    chunk: chunk.clone(),
                    completion: if was_done {
                        ChunkCompletion::Complete
                    } else {
                        ChunkCompletion::Incomplete
                    },
                    id: Some(chunk_id),
                });
            }

            let mut fp = FileProgress {
                relative_path: scanned.relative_path.clone(),
                status: scanned.status,
                skip_reason: scanned.skip_reason.clone(),
                chunks: chunk_progresses,
            };
            fp.status = fp.derive_status();
            files_out.push(fp);
        }

        tx.commit()?;
        Ok((repo_id, RepoProgress { files: files_out }))
    }

    pub fn load_progress(&self, repo_id: i64) -> crate::Result<RepoProgress> {
        let mut files = Vec::new();
        let mut file_stmt = self.conn.prepare(
            "SELECT id, relative_path, skip_reason FROM repo_files WHERE repository_id = ?1 ORDER BY relative_path",
        )?;
        let file_rows: Vec<(i64, String, Option<String>)> = file_stmt
            .query_map(params![repo_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        for (file_id, relative_path, skip_reason) in file_rows {
            let mut chunk_stmt = self.conn.prepare(
                r#"
                SELECT c.id, c.content_hash, c.start_line, c.end_line, c.normalized,
                       COALESCE(p.completed, 0)
                FROM chunks c
                LEFT JOIN chunk_progress p ON p.chunk_id = c.id
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
                chunks,
            };
            fp.status = fp.derive_status();
            files.push(fp);
        }

        Ok(RepoProgress { files })
    }

    /// Record a completed or interrupted session; mark chunk complete when finished.
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
                WHERE f.skip_reason IS NULL
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

fn parse_skip_reason(raw: &str) -> SkipReason {
    if raw == "excluded directory" {
        SkipReason::VcsOrDependencyDir
    } else if raw == "generated or lock file" {
        SkipReason::GeneratedOrLockFile
    } else if raw == "binary file" {
        SkipReason::Binary
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
    use crate::domain::content::{Chunk, ScannedFile, SourceKind};
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
        };
        let (repo_id, progress) = store.sync_scan(&repo, &scan).unwrap();
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
        };
        // Wait — hash of body2 normalized lines differ. Use same normalized hash explicitly.
        let _ = body2;
        let (_, progress2) = store.sync_scan(&repo, &scan2).unwrap();
        assert_eq!(
            progress2.files[0].chunks[0].completion,
            ChunkCompletion::Complete
        );

        let loaded = store.load_progress(repo_id).unwrap();
        assert_eq!(loaded.files[0].derive_status(), FileStatus::Done);
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
        let (older_id, _) = store.sync_scan(&older, &scan).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let (_newer_id, _) = store.sync_scan(&newer, &scan).unwrap();
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
        };
        let (_, progress) = store.sync_scan(&repo, &scan).unwrap();
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
}
