//! Auto-detection of the project's config format for the top-level
//! commands (`audit` / `scan` / `explain` / `graph`) when the user does
//! not name a target file. The precedence mirrors the GitHub Action
//! wrapper's `target` input: compose → actions → gitlab → circleci →
//! dotenv, so `envorigin audit` in any repo just works.

use std::path::{Path, PathBuf};

/// Which backend a project's config belongs to, plus the detected file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Compose { file: PathBuf },
    Actions { file: PathBuf },
    Gitlab { file: PathBuf },
    Circleci { file: PathBuf },
    Dotenv { file: PathBuf },
}

impl Target {
    /// Human-readable backend name, for "detected X format" notices.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Target::Compose { .. } => "compose",
            Target::Actions { .. } => "GitHub Actions",
            Target::Gitlab { .. } => "GitLab CI",
            Target::Circleci { .. } => "CircleCI",
            Target::Dotenv { .. } => "dotenv",
        }
    }

    /// The config file this target resolves to.
    pub fn file(&self) -> &Path {
        match self {
            Target::Compose { file }
            | Target::Actions { file }
            | Target::Gitlab { file }
            | Target::Circleci { file }
            | Target::Dotenv { file } => file,
        }
    }
}

/// File names Compose accepts, in the order they should win.
const COMPOSE_CANDIDATES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yml",
    "docker-compose.yaml",
];

/// Detect the project type inside `dir`. Returns `None` when nothing
/// recognizable is there.
pub fn detect_in(dir: &Path) -> Option<Target> {
    for name in COMPOSE_CANDIDATES {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(Target::Compose { file: candidate });
        }
    }
    // Actions: any workflow file, sorted so the result is deterministic.
    let workflows = dir.join(".github/workflows");
    if let Ok(entries) = std::fs::read_dir(&workflows) {
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .map(|extension| extension == "yml" || extension == "yaml")
                    .unwrap_or(false)
            })
            .collect();
        files.sort();
        if let Some(file) = files.first() {
            return Some(Target::Actions { file: file.clone() });
        }
    }
    let gitlab = dir.join(".gitlab-ci.yml");
    if gitlab.is_file() {
        return Some(Target::Gitlab { file: gitlab });
    }
    let circleci = dir.join(".circleci/config.yml");
    if circleci.is_file() {
        return Some(Target::Circleci { file: circleci });
    }
    let dotenv = dir.join(".env");
    if dotenv.is_file() {
        return Some(Target::Dotenv { file: dotenv });
    }
    None
}

/// The directory a project directory argument resolves to. Files are
/// used as-is; directories are walked by [`detect_in`].
pub fn target_from_path(path: &Path) -> Option<Target> {
    if path.is_file() {
        return Some(Target::Compose {
            file: path.to_path_buf(),
        });
    }
    if path.is_dir() {
        return detect_in(path);
    }
    None
}
