//! Local search helpers for the home Search modal.
//!
//! Accepts GitHub URL / `owner/repo`, or fuzzy-matches Recent and cache names.
//! No network / GitHub API.

use std::fs;
use std::path::Path;

use crate::source::git::parse_github_input;
use crate::store::RecentRepo;

/// A selectable search hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// Text shown in the candidate list.
    pub label: String,
    /// Value passed to `App::open` / resolve.
    pub input: String,
    pub score: u32,
}

/// Modal editing state.
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub query: String,
    pub hits: Vec<SearchHit>,
    pub selected: usize,
}

impl SearchState {
    pub fn refresh(&mut self, recent: &[RecentRepo], cache_dir: &Path) {
        self.hits = build_hits(&self.query, recent, cache_dir);
        if self.hits.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.hits.len() - 1);
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.hits.is_empty() {
            return;
        }
        let len = self.hits.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }

    pub fn selected_input(&self) -> Option<&str> {
        self.hits.get(self.selected).map(|h| h.input.as_str())
    }

    /// Whether Enter may open something (GitHub parse or a listed hit).
    pub fn can_confirm(&self) -> bool {
        if parse_github_input(&self.query).is_some() {
            return true;
        }
        self.selected_input().is_some()
    }

    /// Resolve the input string to open on confirm.
    pub fn confirm_input(&self) -> Option<String> {
        if let Some(gh) = parse_github_input(&self.query) {
            return Some(gh.display_name());
        }
        self.selected_input().map(str::to_string)
    }
}

pub fn build_hits(query: &str, recent: &[RecentRepo], cache_dir: &Path) -> Vec<SearchHit> {
    let q = query.trim();
    let mut hits: Vec<SearchHit> = Vec::new();

    // Exact GitHub form is always offered as the top synthetic hit when it parses.
    if let Some(gh) = parse_github_input(q) {
        hits.push(SearchHit {
            label: format!("{}  (open)", gh.display_name()),
            input: gh.display_name(),
            score: u32::MAX,
        });
    }

    let mut candidates: Vec<(String, String)> = Vec::new();
    for r in recent {
        candidates.push((r.display_name.clone(), r.input.clone()));
    }
    for (label, input) in list_cached_repos(cache_dir) {
        if candidates.iter().any(|(l, _)| l == &label) {
            continue;
        }
        candidates.push((label, input));
    }

    let mut scored: Vec<SearchHit> = candidates
        .into_iter()
        .filter_map(|(label, input)| {
            let score = if q.is_empty() {
                Some(1)
            } else {
                fuzzy_score(q, &label).or_else(|| fuzzy_score(q, &input))
            }?;
            Some(SearchHit {
                label,
                input,
                score,
            })
        })
        .collect();

    scored.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.label.cmp(&b.label)));
    // Keep GitHub synthetic hit first, then scored locals (dedupe by input).
    for hit in scored {
        if hits.iter().any(|h| h.input == hit.input) {
            continue;
        }
        hits.push(hit);
    }
    hits.truncate(20);
    hits
}

/// List `owner/repo` entries under `cache_dir/repos/<owner>__<repo>/`.
pub fn list_cached_repos(cache_dir: &Path) -> Vec<(String, String)> {
    let repos = cache_dir.join("repos");
    let entries = match fs::read_dir(&repos) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some((owner, repo)) = name.split_once("__") {
            if !owner.is_empty() && !repo.is_empty() && entry.path().is_dir() {
                let label = format!("{owner}/{repo}");
                out.push((label.clone(), label));
            }
        }
    }
    out.sort();
    out
}

/// Case-insensitive fuzzy score. Higher is better. `None` = no match.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<u32> {
    let q: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let c: Vec<char> = candidate.chars().flat_map(|c| c.to_lowercase()).collect();
    if q.is_empty() {
        return Some(0);
    }
    // Prefer substring matches.
    let q_str: String = q.iter().collect();
    let c_str: String = c.iter().collect();
    if let Some(pos) = c_str.find(&q_str) {
        let bonus = if pos == 0 { 1000 } else { 500 };
        return Some(bonus + q.len() as u32 * 10);
    }
    // Subsequence match.
    let mut qi = 0;
    let mut gaps = 0u32;
    for ch in &c {
        if qi < q.len() && *ch == q[qi] {
            qi += 1;
        } else if qi > 0 && qi < q.len() {
            gaps += 1;
        }
    }
    if qi == q.len() {
        Some(100 + q.len() as u32 * 10 - gaps.min(99))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::RecentRepo;
    use tempfile::tempdir;

    fn recent(name: &str, input: &str) -> RecentRepo {
        RecentRepo {
            id: 1,
            identity: format!("github:{name}"),
            display_name: name.into(),
            input: input.into(),
            root_path: String::new(),
            last_opened_at: String::new(),
            done_lines: 0,
            total_lines: 10,
        }
    }

    #[test]
    fn github_owner_repo_is_confirmable() {
        let mut s = SearchState {
            query: "rust-lang/mdBook".into(),
            ..Default::default()
        };
        s.refresh(&[], Path::new("/nonexistent"));
        assert!(s.can_confirm());
        assert_eq!(s.confirm_input().as_deref(), Some("rust-lang/mdBook"));
    }

    #[test]
    fn non_github_without_hits_cannot_confirm() {
        let mut s = SearchState {
            query: "zzz-no-match".into(),
            ..Default::default()
        };
        s.refresh(&[], Path::new("/nonexistent"));
        assert!(!s.can_confirm());
        assert!(s.confirm_input().is_none());
    }

    #[test]
    fn fuzzy_prefers_substring() {
        let recent = vec![
            recent("rust-lang/mdBook", "rust-lang/mdBook"),
            recent("foo/bar", "foo/bar"),
        ];
        let hits = build_hits("mdbook", &recent, Path::new("/nonexistent"));
        assert_eq!(hits[0].label, "rust-lang/mdBook");
    }

    #[test]
    fn lists_cache_dirs() {
        let dir = tempdir().unwrap();
        let repos = dir.path().join("repos");
        fs::create_dir_all(repos.join("owner__repo")).unwrap();
        let listed = list_cached_repos(dir.path());
        assert_eq!(listed, vec![("owner/repo".into(), "owner/repo".into())]);
    }
}
