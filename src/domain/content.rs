//! Domain types shared across scan, store, and UI.

use std::path::PathBuf;

/// How a repository was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Local,
    GitHub,
}

/// Resolved repository root ready for scanning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRepository {
    /// Stable identity key used in the database.
    pub identity: String,
    pub display_name: String,
    pub kind: SourceKind,
    /// Absolute path to the working tree (local path or cache clone).
    pub root: PathBuf,
    /// Original user input (URL or path string).
    pub input: String,
}

/// Automatic exclusion reason shown in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    VcsOrDependencyDir,
    GeneratedOrLockFile,
    Binary,
    LineTooLong { max_cols: usize },
    FileTooLarge { max_lines: usize },
    NoChunks,
    IoError(String),
}

impl SkipReason {
    pub fn as_str(&self) -> String {
        match self {
            Self::VcsOrDependencyDir => "excluded directory".into(),
            Self::GeneratedOrLockFile => "generated or lock file".into(),
            Self::Binary => "binary file".into(),
            Self::LineTooLong { max_cols } => format!("line longer than {max_cols} cols"),
            Self::FileTooLarge { max_lines } => format!("more than {max_lines} lines"),
            Self::NoChunks => "no typeable chunks".into(),
            Self::IoError(msg) => format!("read error: {msg}"),
        }
    }
}

/// Effective file status derived from chunks (and skip).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Todo,
    Done,
    Skipped,
}

/// A normalized chunk extracted from a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub relative_path: String,
    /// 1-based inclusive start line in the original file (for display).
    pub start_line: u32,
    /// 1-based inclusive end line in the original file (for display).
    pub end_line: u32,
    /// Normalized body used for typing and hashing.
    pub normalized: String,
    /// SHA-256 hex of `normalized`.
    pub hash: String,
}

/// A file discovered by the scanner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    pub relative_path: String,
    pub status: FileStatus,
    pub skip_reason: Option<SkipReason>,
    pub chunks: Vec<Chunk>,
}

/// Full scan result for a repository root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
    pub files: Vec<ScannedFile>,
}

impl ScanResult {
    pub fn typeable_chunks(&self) -> impl Iterator<Item = &Chunk> {
        self.files
            .iter()
            .filter(|f| f.status != FileStatus::Skipped)
            .flat_map(|f| f.chunks.iter())
    }

    pub fn has_typeable_content(&self) -> bool {
        self.typeable_chunks().next().is_some()
    }
}

/// Progress for a single chunk after store merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChunkCompletion {
    #[default]
    Incomplete,
    Complete,
}

/// Persisted / merged view of a file with chunk progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileProgress {
    pub relative_path: String,
    pub status: FileStatus,
    pub skip_reason: Option<SkipReason>,
    pub chunks: Vec<ChunkProgress>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkProgress {
    pub chunk: Chunk,
    pub completion: ChunkCompletion,
    /// Database id when known.
    pub id: Option<i64>,
}

impl FileProgress {
    pub fn completed_chunks(&self) -> usize {
        self.chunks
            .iter()
            .filter(|c| c.completion == ChunkCompletion::Complete)
            .count()
    }

    pub fn total_chunks(&self) -> usize {
        self.chunks.len()
    }

    pub fn derive_status(&self) -> FileStatus {
        if self.status == FileStatus::Skipped || self.skip_reason.is_some() {
            return FileStatus::Skipped;
        }
        if self.chunks.is_empty() {
            return FileStatus::Skipped;
        }
        if self.completed_chunks() == self.total_chunks() {
            FileStatus::Done
        } else {
            FileStatus::Todo
        }
    }

    pub fn first_incomplete_chunk(&self) -> Option<&ChunkProgress> {
        self.chunks
            .iter()
            .find(|c| c.completion == ChunkCompletion::Incomplete)
    }
}

/// Aggregate repository progress.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoProgress {
    pub files: Vec<FileProgress>,
}

impl RepoProgress {
    pub fn completed_chunks(&self) -> usize {
        self.files.iter().map(FileProgress::completed_chunks).sum()
    }

    pub fn total_chunks(&self) -> usize {
        self.files
            .iter()
            .filter(|f| f.derive_status() != FileStatus::Skipped)
            .map(FileProgress::total_chunks)
            .sum()
    }

    pub fn is_repo_complete(&self) -> bool {
        self.files.iter().all(|f| {
            let s = f.derive_status();
            s == FileStatus::Done || s == FileStatus::Skipped
        }) && self.total_chunks() > 0
    }

    pub fn recommend_path(&self) -> Option<&str> {
        // Prefer partially completed files, else path order of incomplete.
        let mut partial: Option<&str> = None;
        let mut first_todo: Option<&str> = None;
        for f in &self.files {
            if f.derive_status() != FileStatus::Todo {
                continue;
            }
            let done = f.completed_chunks();
            if done > 0 && done < f.total_chunks() {
                return Some(f.relative_path.as_str());
            }
            if partial.is_none() && done > 0 {
                partial = Some(f.relative_path.as_str());
            }
            if first_todo.is_none() {
                first_todo = Some(f.relative_path.as_str());
            }
        }
        partial.or(first_todo)
    }
}

/// Outcome of a typing session for persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub chunk_id: i64,
    pub started_at: String,
    pub ended_at: String,
    pub completed: bool,
    pub keystrokes: u32,
    pub misses: u32,
    pub elapsed_ms: u64,
}

/// Metrics shown on the result screen.
#[derive(Debug, Clone, PartialEq)]
pub struct TypingMetrics {
    pub keystrokes: u32,
    pub misses: u32,
    pub elapsed_ms: u64,
    pub accuracy: f64,
    pub kpm: f64,
    pub wpm: f64,
}

impl TypingMetrics {
    pub fn from_counts(keystrokes: u32, misses: u32, elapsed_ms: u64) -> Self {
        let attempts = keystrokes.saturating_add(misses);
        let accuracy = if attempts == 0 {
            100.0
        } else {
            (f64::from(keystrokes) / f64::from(attempts)) * 100.0
        };
        let minutes = elapsed_ms as f64 / 60_000.0;
        let (kpm, wpm) = if minutes <= f64::EPSILON {
            (0.0, 0.0)
        } else {
            let kpm = f64::from(keystrokes) / minutes;
            let wpm = (f64::from(keystrokes) / 5.0) / minutes;
            (kpm, wpm)
        };
        Self {
            keystrokes,
            misses,
            elapsed_ms,
            accuracy,
            kpm,
            wpm,
        }
    }
}
