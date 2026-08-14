//! Per-file-type include/exclude/hide toggles, keyed by extension or bare filename.

use std::collections::{HashMap, HashSet};

use crate::domain::content::{RepoProgress, SkipReason};

/// Extension (lowercased, with leading `.`) if present, else the bare file name.
pub fn file_type_key(relative_path: &str) -> String {
    key_and_has_extension(relative_path).0
}

/// True when `relative_path`'s file name has no real extension (e.g. `LICENSE`,
/// `Makefile`, or a dotfile like `.gitignore` whose leading dot is not an extension).
pub fn is_extensionless(relative_path: &str) -> bool {
    !key_and_has_extension(relative_path).1
}

fn key_and_has_extension(relative_path: &str) -> (String, bool) {
    let name = relative_path.rsplit('/').next().unwrap_or(relative_path);
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => {
            (format!(".{}", ext.to_ascii_lowercase()), true)
        }
        _ => (name.to_string(), false),
    }
}

/// Tri-state per file type: shown and typeable, shown but excluded (with a
/// reason), or excluded and hidden from the tree entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileTypeState {
    Included,
    Excluded,
    Hidden,
}

impl FileTypeState {
    pub fn is_included(self) -> bool {
        matches!(self, Self::Included)
    }

    pub fn is_hidden(self) -> bool {
        matches!(self, Self::Hidden)
    }

    /// Included -> Excluded -> Hidden -> Included.
    pub fn cycle(self) -> Self {
        match self {
            Self::Included => Self::Excluded,
            Self::Excluded => Self::Hidden,
            Self::Hidden => Self::Included,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Included => "included",
            Self::Excluded => "excluded",
            Self::Hidden => "hidden",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "included" => Some(Self::Included),
            "excluded" => Some(Self::Excluded),
            "hidden" => Some(Self::Hidden),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTypeStat {
    pub key: String,
    pub files: usize,
    pub typeable: usize,
    /// False when every file under this key lacks a real extension
    /// (bare names like `LICENSE`, or dotfiles like `.gitignore`).
    pub has_extension: bool,
}

/// Skip reasons whose files can never be rescued by a file-type toggle;
/// a type made up entirely of these is dropped from the toggle list.
fn is_untoggleable(reason: &SkipReason) -> bool {
    matches!(
        reason,
        SkipReason::VcsOrDependencyDir
            | SkipReason::GeneratedOrLockFile
            | SkipReason::Binary
            | SkipReason::IoError(_)
    )
}

/// File types present in a repository, ordered by file count desc, key asc.
/// Types whose files are all excluded for reasons a toggle cannot affect
/// (binary, generated/lock, vcs dir, io error) are omitted.
pub fn file_type_stats(progress: &RepoProgress) -> Vec<FileTypeStat> {
    let mut stats: HashMap<String, FileTypeStat> = HashMap::new();
    for file in &progress.files {
        let (key, has_extension) = key_and_has_extension(&file.relative_path);
        let entry = stats.entry(key.clone()).or_insert_with(|| FileTypeStat {
            key: key.clone(),
            files: 0,
            typeable: 0,
            has_extension: false,
        });
        entry.files += 1;
        entry.has_extension |= has_extension;
        if !file.chunks.is_empty() {
            entry.typeable += 1;
        }
    }

    let mut droppable: HashMap<String, bool> = HashMap::new();
    for file in &progress.files {
        let key = file_type_key(&file.relative_path);
        let rescuable =
            file.chunks.is_empty() && !file.skip_reason.as_ref().is_some_and(is_untoggleable);
        let entry = droppable.entry(key).or_insert(true);
        if !file.chunks.is_empty() || rescuable {
            *entry = false;
        }
    }

    let mut out: Vec<FileTypeStat> = stats
        .into_values()
        .filter(|stat| !droppable.get(&stat.key).copied().unwrap_or(false))
        .collect();
    out.sort_by(|a, b| b.files.cmp(&a.files).then_with(|| a.key.cmp(&b.key)));
    out
}

/// Per-repository state for file-type keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileTypePrefs(HashMap<String, FileTypeState>);

impl FileTypePrefs {
    pub fn new(entries: HashMap<String, FileTypeState>) -> Self {
        Self(entries)
    }

    pub fn get(&self, key: &str) -> Option<FileTypeState> {
        self.0.get(key).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &FileTypeState)> {
        self.0.iter()
    }
}

/// Relative paths of files whose type is set to `Hidden`.
pub fn hidden_paths(progress: &RepoProgress, prefs: &FileTypePrefs) -> HashSet<String> {
    progress
        .files
        .iter()
        .filter(|f| {
            prefs
                .get(&file_type_key(&f.relative_path))
                .is_some_and(FileTypeState::is_hidden)
        })
        .map(|f| f.relative_path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::content::{Chunk, ChunkProgress, FileProgress, FileStatus};

    #[test]
    fn derives_key_from_extension_or_filename() {
        assert_eq!(file_type_key("src/a.rs"), ".rs");
        assert_eq!(file_type_key("LICENSE"), "LICENSE");
        assert_eq!(file_type_key(".gitignore"), ".gitignore");
        assert_eq!(file_type_key("src/a.test.ts"), ".ts");
    }

    #[test]
    fn extensionless_covers_bare_names_and_dotfiles() {
        assert!(is_extensionless("LICENSE"));
        assert!(is_extensionless("Makefile"));
        assert!(is_extensionless(".gitignore"));
        assert!(!is_extensionless("src/a.rs"));
    }

    fn typeable(path: &str) -> FileProgress {
        FileProgress {
            relative_path: path.into(),
            status: FileStatus::Todo,
            skip_reason: None,
            manual_override: None,
            chunks: vec![ChunkProgress {
                chunk: Chunk {
                    relative_path: path.into(),
                    start_line: 1,
                    end_line: 1,
                    normalized: "x".into(),
                    hash: "h".into(),
                },
                completion: crate::domain::content::ChunkCompletion::Incomplete,
                checkpoint: None,
                id: Some(1),
            }],
        }
    }

    fn skipped(path: &str, reason: SkipReason) -> FileProgress {
        FileProgress {
            relative_path: path.into(),
            status: FileStatus::Skipped,
            skip_reason: Some(reason),
            manual_override: None,
            chunks: Vec::new(),
        }
    }

    #[test]
    fn drops_types_that_cannot_be_rescued() {
        let progress = RepoProgress {
            files: vec![
                skipped("logo.png", SkipReason::GeneratedOrLockFile),
                skipped("Cargo.lock", SkipReason::GeneratedOrLockFile),
                skipped("README.md", SkipReason::ConfigFile),
                typeable("src/main.rs"),
            ],
        };
        let stats = file_type_stats(&progress);
        let keys: Vec<_> = stats.iter().map(|s| s.key.as_str()).collect();
        assert!(!keys.contains(&".png"));
        assert!(!keys.contains(&"Cargo.lock"));
        assert!(keys.contains(&".md"));
        assert!(keys.contains(&".rs"));
    }

    #[test]
    fn orders_by_file_count_desc_then_key_asc() {
        let progress = RepoProgress {
            files: vec![
                typeable("a.rs"),
                typeable("b.rs"),
                typeable("c.ts"),
                typeable("d.md"),
            ],
        };
        let stats = file_type_stats(&progress);
        assert_eq!(stats[0].key, ".rs");
        assert_eq!(stats[0].files, 2);
        assert_eq!(stats[1].key, ".md");
        assert_eq!(stats[2].key, ".ts");
    }

    #[test]
    fn stat_flags_extensionless_types() {
        let progress = RepoProgress {
            files: vec![typeable("LICENSE"), typeable("src/a.rs")],
        };
        let stats = file_type_stats(&progress);
        let license = stats.iter().find(|s| s.key == "LICENSE").unwrap();
        let rs = stats.iter().find(|s| s.key == ".rs").unwrap();
        assert!(!license.has_extension);
        assert!(rs.has_extension);
    }

    #[test]
    fn state_cycles_through_all_three() {
        assert_eq!(FileTypeState::Included.cycle(), FileTypeState::Excluded);
        assert_eq!(FileTypeState::Excluded.cycle(), FileTypeState::Hidden);
        assert_eq!(FileTypeState::Hidden.cycle(), FileTypeState::Included);
    }

    #[test]
    fn hidden_paths_collects_only_hidden_type_files() {
        let progress = RepoProgress {
            files: vec![typeable("a.rs"), typeable("b.md"), typeable("LICENSE")],
        };
        let mut map = HashMap::new();
        map.insert(".md".to_string(), FileTypeState::Hidden);
        let prefs = FileTypePrefs::new(map);
        let hidden = hidden_paths(&progress, &prefs);
        assert_eq!(hidden.len(), 1);
        assert!(hidden.contains("b.md"));
    }
}
