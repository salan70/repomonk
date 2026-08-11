//! Walk a repository tree and classify files.

use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::content::{FileStatus, ScanResult, ScannedFile, SkipReason};
use crate::scan::extract::{extract_chunks, ExtractOptions};

/// Limits used for automatic exclusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkOptions {
    pub max_line_cols: usize,
    pub max_file_lines: usize,
    pub include_tests: bool,
    pub include_configs: bool,
    pub extract: ExtractOptions,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            max_line_cols: 200,
            max_file_lines: 5_000,
            include_tests: false,
            include_configs: false,
            extract: ExtractOptions::default(),
        }
    }
}

const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".jj",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "vendor",
    ".venv",
    "venv",
    "__pycache__",
    ".idea",
    ".vscode",
    ".repomonk",
];

const LOCK_OR_GENERATED: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Gemfile.lock",
    "poetry.lock",
    "composer.lock",
    "go.sum",
    "Cargo.toml.orig",
];

const GENERATED_SUFFIXES: &[&str] = &[
    ".min.js", ".min.css", ".map", ".lock", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico",
    ".pdf", ".zip", ".gz", ".tar", ".wasm", ".so", ".dylib", ".dll", ".exe", ".o", ".a",
];

const TEST_DIR_SEGMENTS: &[&str] = &["test", "tests", "__tests__", "spec", "specs"];

const CONFIG_NAMES: &[&str] = &[
    "Cargo.toml",
    "Cargo.toml.orig",
    "package.json",
    "package-lock.json",
    "tsconfig.json",
    "jsconfig.json",
    "pyproject.toml",
    "setup.cfg",
    "setup.py",
    "Pipfile",
    "poetry.toml",
    "go.mod",
    "go.sum",
    "Gemfile",
    "Rakefile",
    "Makefile",
    "CMakeLists.txt",
    "Dockerfile",
    "docker-compose.yml",
    "docker-compose.yaml",
    ".editorconfig",
    ".gitignore",
    ".gitattributes",
    ".env",
    ".env.example",
    ".eslintrc",
    ".eslintrc.js",
    ".eslintrc.cjs",
    ".eslintrc.json",
    ".prettierrc",
    ".prettierrc.js",
    ".prettierrc.json",
    "prettier.config.js",
    "rust-toolchain",
    "rust-toolchain.toml",
    "clippy.toml",
    "rustfmt.toml",
    ".rustfmt.toml",
];

const CONFIG_EXTENSIONS: &[&str] = &[
    ".toml", ".yaml", ".yml", ".ini", ".cfg", ".conf", ".config", ".jsonc", ".env",
];

/// Scan `root` recursively without following symlinks.
pub fn scan_repository(root: &Path, opts: WalkOptions) -> crate::Result<ScanResult> {
    let mut files = Vec::new();
    walk_dir(root, root, &opts, &mut files)?;
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(ScanResult { files })
}

fn walk_dir(
    root: &Path,
    dir: &Path,
    opts: &WalkOptions,
    out: &mut Vec<ScannedFile>,
) -> crate::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            return Err(crate::Error::Io(err));
        }
    };

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(err) => {
                push_skip(root, &path, SkipReason::IoError(err.to_string()), out);
                continue;
            }
        };

        // Do not follow symlinks.
        if ft.is_symlink() {
            continue;
        }

        if ft.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if EXCLUDED_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk_dir(root, &path, opts, out)?;
            continue;
        }

        if !ft.is_file() {
            continue;
        }

        out.push(classify_file(root, &path, opts));
    }
    Ok(())
}

fn classify_file(root: &Path, path: &Path, opts: &WalkOptions) -> ScannedFile {
    let relative = relative_path(root, path);
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if LOCK_OR_GENERATED.iter().any(|n| name == *n)
        || GENERATED_SUFFIXES.iter().any(|s| name.ends_with(s))
    {
        return skipped(relative, SkipReason::GeneratedOrLockFile);
    }

    // Path segments under excluded dirs are already skipped by walk, but
    // double-check nested names like `foo/node_modules/bar` if called directly.
    if relative.split('/').any(|p| EXCLUDED_DIRS.contains(&p)) {
        return skipped(relative, SkipReason::VcsOrDependencyDir);
    }

    if !opts.include_tests && is_test_path(&relative, &name) {
        return skipped(relative, SkipReason::TestFile);
    }

    if !opts.include_configs && is_config_path(&relative, &name) {
        return skipped(relative, SkipReason::ConfigFile);
    }

    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(err) => return skipped(relative, SkipReason::IoError(err.to_string())),
    };

    if looks_binary(&bytes) {
        return skipped(relative, SkipReason::Binary);
    }

    let text = String::from_utf8_lossy(&bytes);
    let line_count = text.lines().count();
    if line_count > opts.max_file_lines {
        return skipped(
            relative,
            SkipReason::FileTooLarge {
                max_lines: opts.max_file_lines,
            },
        );
    }
    if text.lines().any(|l| l.chars().count() > opts.max_line_cols) {
        return skipped(
            relative,
            SkipReason::LineTooLong {
                max_cols: opts.max_line_cols,
            },
        );
    }

    let chunks = extract_chunks(&relative, &text, opts.extract);
    if chunks.is_empty() {
        return skipped(relative, SkipReason::NoChunks);
    }

    ScannedFile {
        relative_path: relative,
        status: FileStatus::Todo,
        skip_reason: None,
        chunks,
    }
}

fn is_test_path(relative: &str, name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if relative
        .split('/')
        .any(|seg| TEST_DIR_SEGMENTS.contains(&seg.to_ascii_lowercase().as_str()))
    {
        return true;
    }
    if lower.ends_with("_test.rs")
        || lower.ends_with("_test.go")
        || lower.ends_with("_test.py")
        || lower.ends_with("_spec.rb")
        || lower.ends_with("_spec.ts")
        || lower.ends_with("_spec.js")
        || lower.ends_with(".test.ts")
        || lower.ends_with(".test.tsx")
        || lower.ends_with(".test.js")
        || lower.ends_with(".test.jsx")
        || lower.ends_with(".spec.ts")
        || lower.ends_with(".spec.tsx")
        || lower.ends_with(".spec.js")
        || lower.ends_with(".spec.jsx")
        || lower.starts_with("test_")
    {
        return true;
    }
    false
}

fn is_config_path(relative: &str, name: &str) -> bool {
    let _ = relative;
    if CONFIG_NAMES.iter().any(|n| name.eq_ignore_ascii_case(n)) {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    if lower.starts_with(".env") {
        return true;
    }
    CONFIG_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(ext) && !lower.ends_with(".lock"))
}

fn skipped(relative: String, reason: SkipReason) -> ScannedFile {
    ScannedFile {
        relative_path: relative,
        status: FileStatus::Skipped,
        skip_reason: Some(reason),
        chunks: Vec::new(),
    }
}

fn push_skip(root: &Path, path: &Path, reason: SkipReason, out: &mut Vec<ScannedFile>) {
    out.push(skipped(relative_path(root, path), reason));
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn looks_binary(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return true;
    }
    // High ratio of non-text control bytes in the first 8KiB.
    let sample = &bytes[..bytes.len().min(8192)];
    if sample.is_empty() {
        return false;
    }
    let non_text = sample
        .iter()
        .filter(|&&b| b < 7 || (b > 13 && b < 32))
        .count();
    (non_text as f64 / sample.len() as f64) > 0.30
}

/// Resolve a single local file as a one-file repository root (parent) + relative name.
pub fn single_file_scan(file: &Path, opts: WalkOptions) -> crate::Result<(PathBuf, ScanResult)> {
    let file =
        fs::canonicalize(file).map_err(|_| crate::Error::PathNotFound(file.to_path_buf()))?;
    if !file.is_file() {
        return Err(crate::Error::InvalidPath(file));
    }
    let parent = file
        .parent()
        .ok_or_else(|| crate::Error::InvalidPath(file.clone()))?
        .to_path_buf();
    let scanned = classify_file(&parent, &file, &opts);
    Ok((
        parent,
        ScanResult {
            files: vec![scanned],
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn skips_lock_and_binary_and_extracts_source() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("src")).unwrap();
        let mut src = String::new();
        for i in 0..6 {
            src.push_str(&format!("fn f{i}() {{}}\n"));
        }
        fs::write(root.join("src/lib.rs"), src).unwrap();
        fs::write(root.join("Cargo.lock"), "x").unwrap();
        fs::write(root.join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
        fs::create_dir(root.join("node_modules")).unwrap();
        fs::write(root.join("node_modules/x.js"), "var x=1;\n").unwrap();

        let result = scan_repository(root, WalkOptions::default()).unwrap();
        let paths: Vec<_> = result
            .files
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect();
        assert!(paths.contains(&"src/lib.rs"));
        assert!(paths.contains(&"Cargo.lock"));
        assert!(paths.contains(&"blob.bin"));
        assert!(!paths.iter().any(|p| p.starts_with("node_modules")));

        let lib = result
            .files
            .iter()
            .find(|f| f.relative_path == "src/lib.rs")
            .unwrap();
        assert_eq!(lib.status, FileStatus::Todo);
        assert_eq!(lib.chunks.len(), 1);

        let lock = result
            .files
            .iter()
            .find(|f| f.relative_path == "Cargo.lock")
            .unwrap();
        assert_eq!(lock.status, FileStatus::Skipped);
    }

    #[test]
    fn skips_long_lines() {
        let dir = tempdir().unwrap();
        let long = "a".repeat(201);
        fs::write(dir.path().join("long.txt"), format!("{long}\n")).unwrap();
        let result = scan_repository(dir.path(), WalkOptions::default()).unwrap();
        assert_eq!(result.files[0].status, FileStatus::Skipped);
        assert!(matches!(
            result.files[0].skip_reason,
            Some(SkipReason::LineTooLong { .. })
        ));
    }

    #[test]
    fn does_not_follow_symlinks() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret.rs"), "fn x(){}\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(outside.path(), dir.path().join("link")).unwrap();
        }
        let result = scan_repository(dir.path(), WalkOptions::default()).unwrap();
        assert!(
            result.files.is_empty()
                || result
                    .files
                    .iter()
                    .all(|f| !f.relative_path.contains("secret"))
        );
    }

    fn source_body() -> String {
        let mut src = String::new();
        for i in 0..6 {
            src.push_str(&format!("fn f{i}() {{}}\n"));
        }
        src
    }

    #[test]
    fn skips_tests_when_disabled() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), source_body()).unwrap();
        fs::write(root.join("src/lib_test.rs"), source_body()).unwrap();
        fs::create_dir(root.join("tests")).unwrap();
        fs::write(root.join("tests/it.rs"), source_body()).unwrap();

        let skipped = scan_repository(root, WalkOptions::default()).unwrap();
        let test_file = skipped
            .files
            .iter()
            .find(|f| f.relative_path == "src/lib_test.rs")
            .unwrap();
        assert_eq!(test_file.status, FileStatus::Skipped);
        assert_eq!(test_file.skip_reason, Some(SkipReason::TestFile));
        let in_tests = skipped
            .files
            .iter()
            .find(|f| f.relative_path == "tests/it.rs")
            .unwrap();
        assert_eq!(in_tests.skip_reason, Some(SkipReason::TestFile));

        let opts = WalkOptions {
            include_tests: true,
            ..WalkOptions::default()
        };
        let included = scan_repository(root, opts).unwrap();
        assert_eq!(
            included
                .files
                .iter()
                .find(|f| f.relative_path == "src/lib_test.rs")
                .unwrap()
                .status,
            FileStatus::Todo
        );
    }

    #[test]
    fn skips_configs_when_disabled() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), source_body()).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let skipped = scan_repository(root, WalkOptions::default()).unwrap();
        let cargo = skipped
            .files
            .iter()
            .find(|f| f.relative_path == "Cargo.toml")
            .unwrap();
        assert_eq!(cargo.status, FileStatus::Skipped);
        assert_eq!(cargo.skip_reason, Some(SkipReason::ConfigFile));

        let opts = WalkOptions {
            include_configs: true,
            ..WalkOptions::default()
        };
        let included = scan_repository(root, opts).unwrap();
        assert_eq!(
            included
                .files
                .iter()
                .find(|f| f.relative_path == "Cargo.toml")
                .unwrap()
                .status,
            FileStatus::Todo
        );
    }
}
