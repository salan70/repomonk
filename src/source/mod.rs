pub mod git;
pub mod local;

use std::path::Path;

use crate::domain::content::ResolvedRepository;
use crate::source::git::{ensure_github_clone, parse_github_input};
use crate::source::local::{looks_like_local_path, resolve_local};
use crate::Error;

/// Resolve user input to a local working tree.
pub fn resolve_source(
    input: &str,
    cache_root: &Path,
    refresh: bool,
) -> crate::Result<ResolvedRepository> {
    let input = input.trim();
    if input.is_empty() {
        return Err(Error::InvalidInput("empty repository input".into()));
    }

    if looks_like_local_path(input) {
        return resolve_local(input);
    }

    if let Some(gh) = parse_github_input(input) {
        return ensure_github_clone(&gh, cache_root, refresh);
    }

    // Fallback: try local path even if it does not yet exist → clear error.
    if Path::new(input).exists() {
        return resolve_local(input);
    }

    Err(Error::InvalidInput(format!(
        "expected a GitHub URL, owner/repo, or local path; got `{input}`"
    )))
}
