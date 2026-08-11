//! Clone and cache GitHub repositories via system `git`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::domain::content::{ResolvedRepository, SourceKind};
use crate::Error;

/// Parsed GitHub repository reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRef {
    pub owner: String,
    pub repo: String,
}

impl GitHubRef {
    pub fn cache_dir_name(&self) -> String {
        format!("{}__{}", self.owner, self.repo)
    }

    pub fn clone_url_https(&self) -> String {
        format!("https://github.com/{}/{}.git", self.owner, self.repo)
    }

    pub fn display_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    pub fn identity(&self) -> String {
        format!("github:{}/{}", self.owner, self.repo)
    }
}

/// Parse `https://github.com/owner/repo`, `git@github.com:owner/repo.git`, or `owner/repo`.
pub fn parse_github_input(input: &str) -> Option<GitHubRef> {
    let trimmed = input.trim().trim_end_matches('/');

    if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
        return split_owner_repo(rest);
    }
    if let Some(rest) = trimmed.strip_prefix("http://github.com/") {
        return split_owner_repo(rest);
    }
    if let Some(rest) = trimmed.strip_prefix("ssh://git@github.com/") {
        return split_owner_repo(rest);
    }
    if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        return split_owner_repo(rest);
    }

    // Bare owner/repo without extra path segments or scheme.
    if trimmed.contains("://") || trimmed.starts_with("git@") {
        return None;
    }
    split_owner_repo(trimmed)
}

fn split_owner_repo(rest: &str) -> Option<GitHubRef> {
    let rest = rest.trim_end_matches(".git");
    let mut parts = rest.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    if !is_valid_name(owner) || !is_valid_name(repo) {
        return None;
    }
    Some(GitHubRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

fn is_valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Ensure a shallow blob-filtered clone exists under `cache_root`.
pub fn ensure_github_clone(
    gh: &GitHubRef,
    cache_root: &Path,
    refresh: bool,
) -> crate::Result<ResolvedRepository> {
    ensure_git_available()?;

    let repo_cache = cache_root.join("repos").join(gh.cache_dir_name());
    if refresh && repo_cache.exists() {
        remove_path_careful(&repo_cache)?;
    }

    if repo_cache.join(".git").exists() {
        // Reuse cache; optionally fetch — MVP reuses as-is unless --refresh.
        return Ok(resolved(gh, repo_cache));
    }

    if repo_cache.exists() {
        remove_path_careful(&repo_cache)?;
    }
    fs::create_dir_all(repo_cache.parent().unwrap())?;

    let url = gh.clone_url_https();
    let status = Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--filter=blob:none",
            "--",
            &url,
            repo_cache
                .to_str()
                .ok_or_else(|| Error::Message("cache path is not valid UTF-8".into()))?,
        ])
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                Error::GitNotFound
            } else {
                Error::Git(err.to_string())
            }
        })?;

    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr).trim().to_string();
        let msg = if stderr.is_empty() {
            format!("exit {}", status.status)
        } else {
            // Do not persist; only return for display. Truncate aggressively.
            truncate(&stderr, 400)
        };
        // Clean partial clone.
        let _ = fs::remove_dir_all(&repo_cache);
        return Err(Error::GitClone(msg));
    }

    Ok(resolved(gh, repo_cache))
}

fn resolved(gh: &GitHubRef, root: PathBuf) -> ResolvedRepository {
    ResolvedRepository {
        identity: gh.identity(),
        display_name: gh.display_name(),
        kind: SourceKind::GitHub,
        root,
        input: gh.display_name(),
    }
}

fn ensure_git_available() -> crate::Result<()> {
    let out = Command::new("git")
        .args(["--version"])
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                Error::GitNotFound
            } else {
                Error::Git(err.to_string())
            }
        })?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::GitNotFound)
    }
}

fn remove_path_careful(path: &Path) -> crate::Result<()> {
    if path.is_symlink() {
        fs::remove_file(path)?;
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_and_ssh_and_bare() {
        assert_eq!(
            parse_github_input("https://github.com/foo/bar").unwrap(),
            GitHubRef {
                owner: "foo".into(),
                repo: "bar".into()
            }
        );
        assert_eq!(
            parse_github_input("https://github.com/foo/bar.git/")
                .unwrap()
                .repo,
            "bar"
        );
        assert_eq!(
            parse_github_input("git@github.com:foo/bar.git")
                .unwrap()
                .owner,
            "foo"
        );
        assert_eq!(
            parse_github_input("foo/bar").unwrap(),
            GitHubRef {
                owner: "foo".into(),
                repo: "bar".into()
            }
        );
        assert!(parse_github_input("not a repo").is_none());
        assert!(parse_github_input("foo/bar/baz").is_none());
    }
}
