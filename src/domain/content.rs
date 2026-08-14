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
    TestFile,
    ConfigFile,
    FileTypeDisabled,
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
            Self::TestFile => "test file (include_tests=false)".into(),
            Self::ConfigFile => "config or docs (include_configs=false)".into(),
            Self::FileTypeDisabled => "file type disabled".into(),
            Self::NoChunks => "no typeable content".into(),
            Self::IoError(msg) => format!("read error: {msg}"),
        }
    }

    pub fn short_label(&self) -> &'static str {
        match self {
            Self::TestFile => "test",
            Self::GeneratedOrLockFile => "generated",
            Self::ConfigFile => "config",
            Self::Binary => "binary",
            Self::LineTooLong { .. } => "too long",
            Self::FileTooLarge { .. } => "too large",
            Self::FileTypeDisabled => "type off",
            Self::NoChunks => "empty",
            Self::VcsOrDependencyDir => "excluded dir",
            Self::IoError(_) => "read error",
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

/// User override that always wins over automatic skip detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualOverride {
    Skip,
    Include,
}

impl ManualOverride {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Include => "include",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "skip" => Some(Self::Skip),
            "include" => Some(Self::Include),
            _ => None,
        }
    }
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

/// A resolved repository-local import, with source location for flow order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportEdge {
    pub importer: String,
    pub imported: String,
    /// 1-based line of the import / module declaration.
    pub decl_line: usize,
    /// 1-based line where a binding was first used outside import lines.
    pub first_use_line: Option<usize>,
    /// Import statement text for UI.
    pub raw: String,
}

/// Full scan result for a repository root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
    pub files: Vec<ScannedFile>,
    /// Resolved repository-local imports.
    pub import_edges: Vec<ImportEdge>,
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
    pub manual_override: Option<ManualOverride>,
    pub chunks: Vec<ChunkProgress>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkProgress {
    pub chunk: Chunk,
    pub completion: ChunkCompletion,
    pub checkpoint: Option<TypingCheckpoint>,
    /// Database id when known.
    pub id: Option<i64>,
}

/// The latest durable state of an in-progress typing session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypingCheckpoint {
    pub chunk_id: i64,
    /// Character offset into the normalized body, including auto-indent.
    pub cursor: usize,
    pub keystrokes: u32,
    pub misses: u32,
    /// Elapsed time while the typing session was active, excluding suspension.
    pub elapsed_ms: u64,
    /// Original session start timestamp, retained across suspensions.
    pub started_at: String,
    /// Auto-indent behavior used when this session started.
    pub auto_indent: bool,
}

impl TypingCheckpoint {
    /// Count only fully accepted normalized lines before the cursor.
    pub fn progressed_lines(&self, normalized: &str) -> usize {
        let total = normalized_line_count(normalized);
        let prefix: String = normalized.chars().take(self.cursor).collect();
        prefix.chars().filter(|ch| *ch == '\n').count().min(total)
    }
}

impl FileProgress {
    pub fn total_lines(&self) -> usize {
        self.chunks
            .iter()
            .map(|c| normalized_line_count(&c.chunk.normalized))
            .sum()
    }

    pub fn completed_lines(&self) -> usize {
        self.chunks
            .iter()
            .map(|c| {
                if c.completion == ChunkCompletion::Complete {
                    normalized_line_count(&c.chunk.normalized)
                } else {
                    c.checkpoint
                        .as_ref()
                        .map(|checkpoint| checkpoint.progressed_lines(&c.chunk.normalized))
                        .unwrap_or(0)
                }
            })
            .sum()
    }

    pub fn derive_status(&self) -> FileStatus {
        match self.manual_override {
            Some(ManualOverride::Skip) => FileStatus::Skipped,
            Some(ManualOverride::Include) => self.status_from_chunks(),
            None => {
                if self.skip_reason.is_some() {
                    FileStatus::Skipped
                } else {
                    self.status_from_chunks()
                }
            }
        }
    }

    fn status_from_chunks(&self) -> FileStatus {
        if self.chunks.is_empty() {
            return FileStatus::Skipped;
        }
        if self
            .chunks
            .iter()
            .all(|c| c.completion == ChunkCompletion::Complete)
        {
            FileStatus::Done
        } else {
            FileStatus::Todo
        }
    }

    /// Skip reason shown in the tree. Manual skip wins over the automatic reason.
    pub fn display_skip_reason(&self) -> Option<String> {
        if self.derive_status() != FileStatus::Skipped {
            return None;
        }
        if self.manual_override == Some(ManualOverride::Skip) {
            return Some("manually skipped".into());
        }
        self.skip_reason.as_ref().map(|r| r.as_str())
    }

    /// Compact skip label for the tree row (`— test`). Manual skip is `manual`.
    pub fn short_skip_reason(&self) -> Option<&'static str> {
        if self.derive_status() != FileStatus::Skipped {
            return None;
        }
        if self.manual_override == Some(ManualOverride::Skip) {
            return Some("manual");
        }
        self.skip_reason.as_ref().map(SkipReason::short_label)
    }

    /// `x` toggle: skip a typeable file, or include a skipped file that has a body.
    pub fn toggle_override_target(&self) -> Option<ManualOverride> {
        match self.derive_status() {
            FileStatus::Skipped => {
                if self.chunks.is_empty() {
                    None
                } else {
                    Some(ManualOverride::Include)
                }
            }
            FileStatus::Todo | FileStatus::Done => Some(ManualOverride::Skip),
        }
    }

    pub fn first_incomplete_chunk(&self) -> Option<&ChunkProgress> {
        self.chunks
            .iter()
            .find(|c| c.completion == ChunkCompletion::Incomplete)
    }
}

/// Count lines in a normalized body (`""` → 0).
pub fn normalized_line_count(normalized: &str) -> usize {
    if normalized.is_empty() {
        0
    } else {
        normalized.split('\n').count()
    }
}

/// Aggregate repository progress.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoProgress {
    pub files: Vec<FileProgress>,
}

impl RepoProgress {
    pub fn completed_lines(&self) -> usize {
        self.files
            .iter()
            .filter(|f| f.derive_status() != FileStatus::Skipped)
            .map(FileProgress::completed_lines)
            .sum()
    }

    pub fn total_lines(&self) -> usize {
        self.files
            .iter()
            .filter(|f| f.derive_status() != FileStatus::Skipped)
            .map(FileProgress::total_lines)
            .sum()
    }

    pub fn is_repo_complete(&self) -> bool {
        self.files.iter().all(|f| {
            let s = f.derive_status();
            s == FileStatus::Done || s == FileStatus::Skipped
        }) && self.total_lines() > 0
    }

    pub fn recommend_path(&self) -> Option<&str> {
        // Path order of incomplete files (file-unit typing has no partial file).
        self.files
            .iter()
            .find(|f| f.derive_status() == FileStatus::Todo)
            .map(|f| f.relative_path.as_str())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, skipped: bool, body: &str) -> FileProgress {
        FileProgress {
            relative_path: path.into(),
            status: if skipped {
                FileStatus::Skipped
            } else {
                FileStatus::Todo
            },
            skip_reason: skipped.then_some(SkipReason::TestFile),
            manual_override: None,
            chunks: if body.is_empty() {
                Vec::new()
            } else {
                vec![ChunkProgress {
                    chunk: Chunk {
                        relative_path: path.into(),
                        start_line: 1,
                        end_line: 1,
                        normalized: body.into(),
                        hash: "h".into(),
                    },
                    completion: ChunkCompletion::Incomplete,
                    checkpoint: None,
                    id: Some(1),
                }]
            },
        }
    }

    #[test]
    fn manual_skip_wins_over_typeable() {
        let mut f = file("a.rs", false, "fn a() {}");
        f.manual_override = Some(ManualOverride::Skip);
        assert_eq!(f.derive_status(), FileStatus::Skipped);
        assert_eq!(f.display_skip_reason().as_deref(), Some("manually skipped"));
        assert_eq!(f.short_skip_reason(), Some("manual"));
    }

    #[test]
    fn manual_include_wins_over_auto_skip_when_body_exists() {
        let mut f = file("a.rs", true, "fn a() {}");
        f.manual_override = Some(ManualOverride::Include);
        assert_eq!(f.derive_status(), FileStatus::Todo);
        assert!(f.display_skip_reason().is_none());
    }

    #[test]
    fn toggle_skips_typeable_and_includes_skipped_with_body() {
        let typeable = file("a.rs", false, "fn a() {}");
        assert_eq!(
            typeable.toggle_override_target(),
            Some(ManualOverride::Skip)
        );
        let skipped = file("a.rs", true, "fn a() {}");
        assert_eq!(
            skipped.toggle_override_target(),
            Some(ManualOverride::Include)
        );
        let empty = file("bin", true, "");
        assert_eq!(empty.toggle_override_target(), None);
    }

    #[test]
    fn partial_checkpoint_counts_only_completed_lines() {
        let mut f = file("a.rs", false, "one\ntwo\nthree");
        f.chunks[0].checkpoint = Some(TypingCheckpoint {
            chunk_id: 1,
            cursor: 5,
            keystrokes: 5,
            misses: 0,
            elapsed_ms: 10,
            started_at: "t0".into(),
            auto_indent: false,
        });
        assert_eq!(f.completed_lines(), 1);
        assert_eq!(f.derive_status(), FileStatus::Todo);

        f.chunks[0].checkpoint.as_mut().unwrap().cursor = 3;
        assert_eq!(f.completed_lines(), 0);
    }
}
