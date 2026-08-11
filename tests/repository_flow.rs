use std::path::PathBuf;

use repomonk::app::headless::{complete_recommended, open_local};
use repomonk::domain::content::{ChunkCompletion, FileStatus, SessionSummary};
use repomonk::scan::extract::{extract_chunks, hash_normalized, ExtractOptions};
use repomonk::scan::walk::{scan_repository, WalkOptions};
use repomonk::source::git::parse_github_input;
use repomonk::store::SqliteStore;
use tempfile::tempdir;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample_repo")
}

#[test]
fn scan_fixture_marks_skips_and_chunks() {
    let scan = scan_repository(&fixture_root(), WalkOptions::default()).unwrap();
    let lock = scan
        .files
        .iter()
        .find(|f| f.relative_path == "Cargo.lock")
        .unwrap();
    assert_eq!(lock.status, FileStatus::Skipped);

    let bin = scan
        .files
        .iter()
        .find(|f| f.relative_path == "blob.bin")
        .unwrap();
    assert_eq!(bin.status, FileStatus::Skipped);

    let long = scan
        .files
        .iter()
        .find(|f| f.relative_path == "long_line.txt")
        .unwrap();
    assert_eq!(long.status, FileStatus::Skipped);

    let hello = scan
        .files
        .iter()
        .find(|f| f.relative_path == "src/hello.rs")
        .unwrap();
    assert_eq!(hello.status, FileStatus::Todo);
    assert!(!hello.chunks.is_empty());
    assert!(scan.has_typeable_content());
}

#[test]
fn hash_carryover_across_line_shift() {
    let body = "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n";
    let c1 = extract_chunks("f.rs", body, ExtractOptions::default());
    assert_eq!(c1.len(), 1);
    let shifted = format!("\n\n{body}");
    let c2 = extract_chunks("f.rs", &shifted, ExtractOptions::default());
    assert_eq!(c1[0].hash, c2[0].hash);
    assert_ne!(c1[0].start_line, c2[0].start_line);
    assert_eq!(c1[0].hash, hash_normalized(&c1[0].normalized));
}

#[test]
fn headless_complete_persists_and_reloads() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("t.db");
    let cache = dir.path().join("cache");
    std::fs::create_dir_all(&cache).unwrap();

    let fixture = fixture_root();
    let mut app = open_local(fixture.to_str().unwrap(), &db, &cache).unwrap();
    assert!(app.progress().total_chunks() > 0);
    let before = app.progress().completed_chunks();
    complete_recommended(&mut app).unwrap();
    assert_eq!(app.progress().completed_chunks(), before + 1);

    let app2 = open_local(fixture.to_str().unwrap(), &db, &cache).unwrap();
    assert!(app2.progress().completed_chunks() >= 1);
    assert!(app2
        .progress()
        .files
        .iter()
        .flat_map(|f| f.chunks.iter())
        .any(|c| c.completion == ChunkCompletion::Complete));
}

#[test]
fn interrupt_session_does_not_mark_complete() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("t.db");
    let mut store = SqliteStore::open(&db).unwrap();

    let scan = scan_repository(&fixture_root(), WalkOptions::default()).unwrap();
    let repo = repomonk::domain::content::ResolvedRepository {
        identity: "local:fixture".into(),
        display_name: "fixture".into(),
        kind: repomonk::domain::content::SourceKind::Local,
        root: fixture_root(),
        input: fixture_root().to_string_lossy().into(),
    };
    let (repo_id, progress) = store.sync_scan(&repo, &scan).unwrap();
    let chunk_id = progress
        .files
        .iter()
        .flat_map(|f| f.chunks.iter())
        .find(|c| c.completion == ChunkCompletion::Incomplete)
        .and_then(|c| c.id)
        .unwrap();

    store
        .record_session(&SessionSummary {
            chunk_id,
            started_at: "t0".into(),
            ended_at: "t1".into(),
            completed: false,
            keystrokes: 3,
            misses: 1,
            elapsed_ms: 100,
        })
        .unwrap();

    let loaded = store.load_progress(repo_id).unwrap();
    let still = loaded
        .files
        .iter()
        .flat_map(|f| f.chunks.iter())
        .find(|c| c.id == Some(chunk_id))
        .unwrap();
    assert_eq!(still.completion, ChunkCompletion::Incomplete);
}

#[test]
fn parses_github_refs() {
    assert!(parse_github_input("https://github.com/salan70/repomonk").is_some());
    assert!(parse_github_input("salan70/repomonk").is_some());
}
