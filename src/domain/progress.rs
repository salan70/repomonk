//! Pure progress aggregation helpers.

use crate::domain::content::{FileProgress, FileStatus, RepoProgress};

/// Directory node progress for tree display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirProgress {
    pub path: String,
    pub completed_chunks: usize,
    pub total_chunks: usize,
    pub done_files: usize,
    pub todo_files: usize,
    pub skipped_files: usize,
}

/// Aggregate chunk counts under a directory prefix (`""` = repo root).
pub fn directory_progress(repo: &RepoProgress, dir_prefix: &str) -> DirProgress {
    let prefix = if dir_prefix.is_empty() {
        String::new()
    } else {
        format!("{}/", dir_prefix.trim_end_matches('/'))
    };

    let mut completed = 0usize;
    let mut total = 0usize;
    let mut done_files = 0usize;
    let mut todo_files = 0usize;
    let mut skipped_files = 0usize;

    for f in &repo.files {
        let under = if prefix.is_empty() {
            true
        } else {
            f.relative_path.starts_with(&prefix) || f.relative_path == dir_prefix
        };
        if !under {
            continue;
        }
        match f.derive_status() {
            FileStatus::Skipped => skipped_files += 1,
            FileStatus::Done => {
                done_files += 1;
                completed += f.completed_chunks();
                total += f.total_chunks();
            }
            FileStatus::Todo => {
                todo_files += 1;
                completed += f.completed_chunks();
                total += f.total_chunks();
            }
        }
    }

    DirProgress {
        path: dir_prefix.to_string(),
        completed_chunks: completed,
        total_chunks: total,
        done_files,
        todo_files,
        skipped_files,
    }
}

/// Flatten directory names that appear in file paths.
pub fn collect_directories(repo: &RepoProgress) -> Vec<String> {
    let mut dirs = Vec::new();
    for f in &repo.files {
        let mut acc = String::new();
        let parts: Vec<&str> = f.relative_path.split('/').collect();
        if parts.len() <= 1 {
            continue;
        }
        for part in &parts[..parts.len() - 1] {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(part);
            if !dirs.iter().any(|d| d == &acc) {
                dirs.push(acc.clone());
            }
        }
    }
    dirs.sort();
    dirs
}

/// Whether a file is partially complete.
pub fn is_partial(file: &FileProgress) -> bool {
    let done = file.completed_chunks();
    done > 0 && done < file.total_chunks() && file.derive_status() == FileStatus::Todo
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::content::{
        Chunk, ChunkCompletion, ChunkProgress, FileProgress, FileStatus, RepoProgress, SkipReason,
    };

    fn chunk(path: &str, hash: &str) -> Chunk {
        Chunk {
            relative_path: path.into(),
            start_line: 1,
            end_line: 1,
            normalized: "x".into(),
            hash: hash.into(),
        }
    }

    #[test]
    fn directory_rolls_up_chunks() {
        let repo = RepoProgress {
            files: vec![
                FileProgress {
                    relative_path: "src/a.rs".into(),
                    status: FileStatus::Todo,
                    skip_reason: None,
                    chunks: vec![ChunkProgress {
                        chunk: chunk("src/a.rs", "1"),
                        completion: ChunkCompletion::Complete,
                        id: Some(1),
                    }],
                },
                FileProgress {
                    relative_path: "src/b.rs".into(),
                    status: FileStatus::Todo,
                    skip_reason: None,
                    chunks: vec![ChunkProgress {
                        chunk: chunk("src/b.rs", "2"),
                        completion: ChunkCompletion::Incomplete,
                        id: Some(2),
                    }],
                },
                FileProgress {
                    relative_path: "README.md".into(),
                    status: FileStatus::Skipped,
                    skip_reason: Some(SkipReason::NoChunks),
                    chunks: vec![],
                },
            ],
        };
        let d = directory_progress(&repo, "src");
        assert_eq!(d.completed_chunks, 1);
        assert_eq!(d.total_chunks, 2);
        assert_eq!(d.done_files, 1);
        assert_eq!(d.todo_files, 1);
    }
}
