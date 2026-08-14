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
fn scan_fixture_marks_skips_and_file_bodies() {
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
    assert_eq!(hello.chunks.len(), 1);
    assert!(scan.has_typeable_content());
}

#[test]
fn hash_carryover_when_dropped_lines_shift_numbers() {
    let body = "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n";
    let c1 = extract_chunks("f.rs", body, ExtractOptions::default());
    assert_eq!(c1.len(), 1);
    // Non-ASCII lines are dropped before hashing, so display line numbers shift
    // while the normalized body (and hash) stay the same.
    let shifted = format!("コメント\n別コメント\n{body}");
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
    assert!(app.progress().total_lines() > 0);
    let before = app.progress().completed_lines();
    let before_files = app
        .progress()
        .files
        .iter()
        .filter(|f| f.derive_status() == FileStatus::Done)
        .count();
    complete_recommended(&mut app).unwrap();
    assert!(app.progress().completed_lines() > before);
    assert_eq!(
        app.progress()
            .files
            .iter()
            .filter(|f| f.derive_status() == FileStatus::Done)
            .count(),
        before_files + 1
    );

    let app2 = open_local(fixture.to_str().unwrap(), &db, &cache).unwrap();
    assert!(app2.progress().completed_lines() >= 1);
    assert!(app2
        .progress()
        .files
        .iter()
        .any(|f| f.derive_status() == FileStatus::Done));
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
    let (repo_id, progress) = store.sync_scan(&repo, &scan, true).unwrap();
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

#[test]
fn line_filter_settings_change_extracted_body() {
    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.rs"),
        "use crate::dep;\n/// docs\n// note\nfn main() {}\n",
    )
    .unwrap();

    let default_scan = scan_repository(dir.path(), WalkOptions::default()).unwrap();
    let default_body = &default_scan.files[0].chunks[0].normalized;
    assert_eq!(default_body, "/// docs\nfn main() {}\n");

    let mut options = WalkOptions::default();
    options.extract.include_imports = true;
    options.extract.include_comments = true;
    let all_scan = scan_repository(dir.path(), options).unwrap();
    assert_eq!(
        all_scan.files[0].chunks[0].normalized,
        "use crate::dep;\n/// docs\n// note\nfn main() {}\n"
    );
}

#[test]
fn scan_collects_repository_local_dependency_edges() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/main.ts"),
        "import { dep } from './dep';\nexport const main = dep;\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/dep.ts"), "export const dep = 1;\n").unwrap();
    std::fs::write(
        dir.path().join("src/unrelated.ts"),
        "export const unrelated = 2;\n",
    )
    .unwrap();

    let scan = scan_repository(dir.path(), WalkOptions::default()).unwrap();
    assert_eq!(scan.import_edges.len(), 1);
    assert_eq!(scan.import_edges[0].importer, "src/main.ts");
    assert_eq!(scan.import_edges[0].imported, "src/dep.ts");
    assert_eq!(scan.import_edges[0].decl_line, 1);
    assert_eq!(scan.import_edges[0].first_use_line, Some(2));
}

#[test]
fn rust_flow_starts_at_main_and_recommend_follows() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "mod util;\nfn main() {\n    util::run();\n}\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/util.rs"), "pub fn run() {}\n").unwrap();

    let state = tempdir().unwrap();
    let mut user = repomonk::config::UserConfig::default();
    user.progress.mode = repomonk::config::ProgressMode::Flow;
    let mut app = repomonk::app::headless::open_local_with_user_config(
        dir.path().to_str().unwrap(),
        &state.path().join("db.sqlite"),
        &state.path().join("cache"),
        user,
    )
    .unwrap();

    let paths = repomonk::app::headless::flow_paths(&app).expect("flow order");
    assert_eq!(paths[0], "src/main.rs");
    assert_eq!(paths[1], "src/util.rs");
    assert_eq!(
        repomonk::app::headless::recommend_path(&app).as_deref(),
        Some("src/main.rs")
    );

    complete_recommended(&mut app).unwrap();
    assert_eq!(
        repomonk::app::headless::recommend_path(&app).as_deref(),
        Some("src/util.rs")
    );

    let mut user_manual = repomonk::config::UserConfig::default();
    user_manual.progress.mode = repomonk::config::ProgressMode::Manual;
    let reopened = repomonk::app::headless::open_local_with_user_config(
        dir.path().to_str().unwrap(),
        &state.path().join("db.sqlite"),
        &state.path().join("cache"),
        user_manual,
    )
    .unwrap();
    assert_eq!(
        repomonk::app::headless::flow_entry(&reopened).as_deref(),
        Some("src/main.rs")
    );
    assert_eq!(
        repomonk::app::headless::flow_paths(&reopened).as_deref(),
        Some(paths.as_slice())
    );
}

#[test]
fn typescript_flow_starts_at_index() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("package.json"),
        r#"{ "name": "demo", "main": "src/index.ts" }"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/index.ts"),
        "import { x } from './x';\nx();\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/x.ts"), "export function x() {}\n").unwrap();

    let state = tempdir().unwrap();
    let mut user = repomonk::config::UserConfig::default();
    user.progress.mode = repomonk::config::ProgressMode::Flow;
    let app = repomonk::app::headless::open_local_with_user_config(
        dir.path().to_str().unwrap(),
        &state.path().join("db.sqlite"),
        &state.path().join("cache"),
        user,
    )
    .unwrap();
    let paths = repomonk::app::headless::flow_paths(&app).expect("flow order");
    assert_eq!(paths[0], "src/index.ts");
    assert_eq!(paths[1], "src/x.ts");
}

#[test]
fn python_flow_starts_at_main() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("main.py"), "from util import run\nrun()\n").unwrap();
    std::fs::write(dir.path().join("util.py"), "def run():\n    pass\n").unwrap();

    let state = tempdir().unwrap();
    let mut user = repomonk::config::UserConfig::default();
    user.progress.mode = repomonk::config::ProgressMode::Flow;
    let app = repomonk::app::headless::open_local_with_user_config(
        dir.path().to_str().unwrap(),
        &state.path().join("db.sqlite"),
        &state.path().join("cache"),
        user,
    )
    .unwrap();
    let paths = repomonk::app::headless::flow_paths(&app).expect("flow order");
    assert_eq!(paths[0], "main.py");
    assert_eq!(paths[1], "util.py");
}
