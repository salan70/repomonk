//! Resolve a local path into a repository root.

use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::content::{ResolvedRepository, SourceKind};
use crate::Error;

/// Resolve a local filesystem path (file or directory).
pub fn resolve_local(input: &str) -> crate::Result<ResolvedRepository> {
    let path = PathBuf::from(input);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };

    let path = fs::canonicalize(&path).map_err(|_| Error::PathNotFound(path))?;

    if path.is_file() {
        let parent = path
            .parent()
            .ok_or_else(|| Error::InvalidPath(path.clone()))?
            .to_path_buf();
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into());
        return Ok(ResolvedRepository {
            identity: format!("local:{}", path.to_string_lossy()),
            display_name: name,
            kind: SourceKind::Local,
            root: parent,
            input: input.to_string(),
        });
    }

    if path.is_dir() {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        return Ok(ResolvedRepository {
            identity: format!("local:{}", path.to_string_lossy()),
            display_name: name,
            kind: SourceKind::Local,
            root: path,
            input: input.to_string(),
        });
    }

    Err(Error::InvalidPath(path))
}

/// True when `input` looks like an existing local path rather than a GitHub ref.
pub fn looks_like_local_path(input: &str) -> bool {
    let p = Path::new(input);
    input.starts_with('.')
        || input.starts_with('/')
        || input.starts_with('~')
        || p.exists()
        || input.contains('\\')
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn resolves_directory() {
        let dir = tempdir().unwrap();
        let r = resolve_local(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(r.kind, SourceKind::Local);
        assert_eq!(r.root, fs::canonicalize(dir.path()).unwrap());
    }

    #[test]
    fn resolves_file_as_parent_root() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("only.rs");
        fs::write(&file, "fn main(){}\n").unwrap();
        let r = resolve_local(file.to_str().unwrap()).unwrap();
        assert_eq!(r.root, fs::canonicalize(dir.path()).unwrap());
        assert!(r.identity.contains("only.rs"));
    }
}
