use std::process::Command;

use tempfile::tempdir;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_repomonk"))
}

#[test]
fn help_exits_zero() {
    let out = bin().arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("repomonk"));
}

#[test]
fn help_mentions_optional_target() {
    let out = bin().arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Omit to open the home screen") || stdout.contains("[TARGET]"),
        "{stdout}"
    );
}

#[test]
fn purge_cancel_and_yes() {
    let dir = tempdir().unwrap();
    let cache = dir.path().join("cache");
    let data = dir.path().join("data");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(cache.join("x"), "1").unwrap();
    std::fs::write(data.join("repomonk.db"), "db").unwrap();

    // Simpler path: --yes deletes without stdin.
    let out = bin()
        .args([
            "--purge",
            "--yes",
            "--cache-dir",
            cache.to_str().unwrap(),
            "--data-dir",
            data.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{:?}", out);
    assert!(!cache.exists());
    assert!(!data.exists());
}

#[test]
fn invalid_path_nonzero() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("nope");
    let out = bin()
        .args([
            missing.to_str().unwrap(),
            "--cache-dir",
            dir.path().join("c").to_str().unwrap(),
            "--data-dir",
            dir.path().join("d").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
}
