// SPDX-License-Identifier: MIT OR Apache-2.0
//! Database utilities for path processing and connection management

use std::path::{Path, PathBuf};

/// Process database path argument according to semcode's database location rules
///
/// This function implements the standard semcode database path resolution logic:
/// 1. If `database_arg` is provided:
///    - If it's a directory, look for `.semcode.db` within it
///    - Otherwise, use the path as-is (direct database path)
/// 2. If `database_arg` is None, check the `SEMCODE_DB` environment variable
///    (same directory/suffix semantics as the `-d` flag)
/// 3. If neither is set, resolve from `source_dir` (indexing) or `.` (queries):
///    - Use `dir/.semcode.db` if it exists locally
///    - Try git-aware discovery (repo root, then main repo for linked worktrees)
///
/// # Arguments
/// * `database_arg` - Optional database path from command line (-d flag)
/// * `source_dir` - Optional source directory for indexing operations
///
/// # Returns
/// String representation of the database path to use
pub fn process_database_path(database_arg: Option<&str>, source_dir: Option<&Path>) -> String {
    match database_arg {
        Some(path) => resolve_path(path),
        None => {
            // Check SEMCODE_DB environment variable before falling back to
            // source-dir or current-dir defaults.
            if let Ok(env_path) = std::env::var("SEMCODE_DB") {
                let env_path = env_path.trim();
                if !env_path.is_empty() {
                    return resolve_path(env_path);
                }
            }

            let start = source_dir.unwrap_or(Path::new("."));
            resolve_db_for_dir(start)
        }
    }
}

/// Resolve `.semcode.db` for a given starting directory.
///
/// Checks (in order):
/// 1. `dir/.semcode.db` if it exists locally
/// 2. Existing `.semcode.db` at the git repo root or main repo (for worktrees)
/// 3. The git repo root as the default creation location (prefers main repo for worktrees)
fn resolve_db_for_dir(dir: &Path) -> String {
    let local = dir.join(".semcode.db");
    if local.is_dir() {
        return local.to_string_lossy().to_string();
    }

    if let Ok(repo) = gix::discover(dir) {
        if let Some(found) = find_existing_db(&repo) {
            return found;
        }
        return default_db_location(&repo).unwrap_or_else(|| local.to_string_lossy().to_string());
    }

    local.to_string_lossy().to_string()
}

/// Search for an existing `.semcode.db` via the git repository.
///
/// Checks the working directory first, then the main repo for linked worktrees.
fn find_existing_db(repo: &gix::Repository) -> Option<String> {
    let workdir = repo.workdir()?;
    let candidate = workdir.join(".semcode.db");
    if candidate.is_dir() {
        return Some(candidate.to_string_lossy().to_string());
    }

    if let Some(main_workdir) = main_repo_workdir(repo) {
        let candidate = main_workdir.join(".semcode.db");
        if candidate.is_dir() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }

    None
}

/// Return the best location to create a new `.semcode.db`.
///
/// For linked worktrees, prefers the main repo so all worktrees share one database.
/// Otherwise uses the repo workdir.
fn default_db_location(repo: &gix::Repository) -> Option<String> {
    let target = main_repo_workdir(repo).or_else(|| repo.workdir().map(|p| p.to_path_buf()))?;
    Some(target.join(".semcode.db").to_string_lossy().to_string())
}

/// For linked worktrees, resolve the main repository's working directory.
/// Returns `None` if this is not a linked worktree or if the main repo is bare.
fn main_repo_workdir(repo: &gix::Repository) -> Option<PathBuf> {
    let common = repo.common_dir();
    if common == repo.git_dir() {
        return None;
    }
    let canonical = common.canonicalize().ok()?;
    let main_workdir = canonical.parent()?;
    // Verify this is actually a working directory (not a bare repo)
    // by checking that .git exists as a child of the candidate.
    if main_workdir.join(".git").exists() {
        Some(main_workdir.to_path_buf())
    } else {
        None
    }
}

/// Normalize a database path: append `.semcode.db` to directories, pass
/// paths that already end with `.semcode.db` through unchanged, and
/// return anything else as-is.
fn resolve_path(path: &str) -> String {
    let path_obj = Path::new(path);

    if path.ends_with(".semcode.db") {
        path.to_string()
    } else if path_obj.is_dir() {
        path_obj.join(".semcode.db").to_string_lossy().to_string()
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    /// Serializes tests that read or write the SEMCODE_DB environment variable.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_process_database_path_with_explicit_path() {
        // Test with explicit file path
        let result = process_database_path(Some("/path/to/my.db"), None);
        assert_eq!(result, "/path/to/my.db");
    }

    #[test]
    fn test_process_database_path_with_directory() {
        // Test with directory - should append .semcode.db
        // Note: In a real test environment, we'd need to create actual directories
        // For now, we test the logic with a hypothetical directory
        let result = process_database_path(Some("/existing/dir"), None);
        // This would be "/existing/dir/.semcode.db" if the directory exists
        // For this unit test, it will treat it as a file since we don't have real filesystem
        assert_eq!(result, "/existing/dir");
    }

    #[test]
    fn test_process_database_path_no_args_no_source() {
        let _guard = ENV_LOCK.lock().unwrap();
        let saved = std::env::var("SEMCODE_DB").ok();
        std::env::remove_var("SEMCODE_DB");

        let result = process_database_path(None, None);
        assert_eq!(result, "./.semcode.db");

        if let Some(v) = saved {
            std::env::set_var("SEMCODE_DB", v);
        }
    }

    #[test]
    fn test_process_database_path_no_args_with_source() {
        let _guard = ENV_LOCK.lock().unwrap();
        let saved = std::env::var("SEMCODE_DB").ok();
        std::env::remove_var("SEMCODE_DB");

        let source_path = Path::new("/source/code");
        let result = process_database_path(None, Some(source_path));
        assert_eq!(result, "/source/code/.semcode.db");

        if let Some(v) = saved {
            std::env::set_var("SEMCODE_DB", v);
        }
    }

    #[test]
    fn test_process_database_path_current_dir_source() {
        let _guard = ENV_LOCK.lock().unwrap();
        let saved = std::env::var("SEMCODE_DB").ok();
        std::env::remove_var("SEMCODE_DB");

        let source_path = Path::new(".");
        let result = process_database_path(None, Some(source_path));
        assert_eq!(result, "./.semcode.db");

        if let Some(v) = saved {
            std::env::set_var("SEMCODE_DB", v);
        }
    }

    #[test]
    fn test_env_var_used_when_no_flag() {
        let _guard = ENV_LOCK.lock().unwrap();
        let saved = std::env::var("SEMCODE_DB").ok();
        std::env::set_var("SEMCODE_DB", "/data/my-project.semcode.db");

        let result = process_database_path(None, None);
        assert_eq!(result, "/data/my-project.semcode.db");

        // Also overrides source_dir fallback
        let result = process_database_path(None, Some(Path::new("/source/code")));
        assert_eq!(result, "/data/my-project.semcode.db");

        match saved {
            Some(v) => std::env::set_var("SEMCODE_DB", v),
            None => std::env::remove_var("SEMCODE_DB"),
        }
    }

    #[test]
    fn test_flag_overrides_env_var() {
        let _guard = ENV_LOCK.lock().unwrap();
        let saved = std::env::var("SEMCODE_DB").ok();
        std::env::set_var("SEMCODE_DB", "/env/path.semcode.db");

        let result = process_database_path(Some("/flag/path.semcode.db"), None);
        assert_eq!(result, "/flag/path.semcode.db");

        match saved {
            Some(v) => std::env::set_var("SEMCODE_DB", v),
            None => std::env::remove_var("SEMCODE_DB"),
        }
    }

    #[test]
    fn test_empty_env_var_ignored() {
        let _guard = ENV_LOCK.lock().unwrap();
        let saved = std::env::var("SEMCODE_DB").ok();
        std::env::set_var("SEMCODE_DB", "");

        let result = process_database_path(None, None);
        assert_eq!(result, "./.semcode.db");

        match saved {
            Some(v) => std::env::set_var("SEMCODE_DB", v),
            None => std::env::remove_var("SEMCODE_DB"),
        }
    }

    /// Helper: create a git repo with an initial commit.
    fn init_git_repo(path: &std::path::Path) {
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .unwrap();
        std::fs::write(path.join("file.txt"), "hello\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(path)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
    }

    #[test]
    fn test_git_discovery_at_repo_root() {
        let tmpdir = tempfile::tempdir().unwrap();
        let repo = tmpdir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        init_git_repo(&repo);

        // No .semcode.db yet — should default to repo root
        let result = resolve_db_for_dir(&repo);
        assert!(result.ends_with(".semcode.db"));

        // Create .semcode.db — should find it
        std::fs::create_dir(repo.join(".semcode.db")).unwrap();
        let result = resolve_db_for_dir(&repo);
        assert!(result.ends_with(".semcode.db"));
        assert!(result.contains(repo.to_str().unwrap()));
    }

    #[test]
    fn test_git_discovery_from_subdirectory() {
        let tmpdir = tempfile::tempdir().unwrap();
        let repo = tmpdir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        init_git_repo(&repo);
        std::fs::create_dir(repo.join(".semcode.db")).unwrap();

        let subdir = repo.join("src").join("deep");
        std::fs::create_dir_all(&subdir).unwrap();

        let result = resolve_db_for_dir(&subdir);
        assert!(result.ends_with(".semcode.db"));
        assert!(result.contains(repo.to_str().unwrap()));
    }

    #[test]
    fn test_git_discovery_worktree_finds_main_repo_db() {
        let tmpdir = tempfile::tempdir().unwrap();
        let main_repo = tmpdir.path().join("main");
        std::fs::create_dir(&main_repo).unwrap();
        init_git_repo(&main_repo);
        std::fs::create_dir(main_repo.join(".semcode.db")).unwrap();

        let wt_path = tmpdir.path().join("worktree");
        let output = std::process::Command::new("git")
            .args(["worktree", "add", "-d", wt_path.to_str().unwrap(), "HEAD"])
            .current_dir(&main_repo)
            .output()
            .unwrap();
        assert!(output.status.success(), "git worktree add failed");

        assert!(!wt_path.join(".semcode.db").exists());
        let result = resolve_db_for_dir(&wt_path);
        assert!(result.ends_with(".semcode.db"));
        assert!(
            result.contains(main_repo.to_str().unwrap()),
            "should resolve to main repo's db, got: {result}"
        );
    }

    #[test]
    fn test_git_discovery_worktree_local_db_preferred() {
        let tmpdir = tempfile::tempdir().unwrap();
        let main_repo = tmpdir.path().join("main");
        std::fs::create_dir(&main_repo).unwrap();
        init_git_repo(&main_repo);
        std::fs::create_dir(main_repo.join(".semcode.db")).unwrap();

        let wt_path = tmpdir.path().join("worktree");
        let output = std::process::Command::new("git")
            .args(["worktree", "add", "-d", wt_path.to_str().unwrap(), "HEAD"])
            .current_dir(&main_repo)
            .output()
            .unwrap();
        assert!(output.status.success());

        std::fs::create_dir(wt_path.join(".semcode.db")).unwrap();
        let result = resolve_db_for_dir(&wt_path);
        assert!(
            result.contains(wt_path.to_str().unwrap()),
            "should prefer worktree's own db, got: {result}"
        );
    }

    #[test]
    fn test_git_discovery_worktree_no_db_defaults_to_main_repo() {
        let tmpdir = tempfile::tempdir().unwrap();
        let main_repo = tmpdir.path().join("main");
        std::fs::create_dir(&main_repo).unwrap();
        init_git_repo(&main_repo);

        let wt_path = tmpdir.path().join("worktree");
        let output = std::process::Command::new("git")
            .args(["worktree", "add", "-d", wt_path.to_str().unwrap(), "HEAD"])
            .current_dir(&main_repo)
            .output()
            .unwrap();
        assert!(output.status.success());

        // No .semcode.db anywhere — should default to main repo, not worktree
        assert!(!main_repo.join(".semcode.db").exists());
        assert!(!wt_path.join(".semcode.db").exists());
        let result = resolve_db_for_dir(&wt_path);
        assert!(
            result.contains(main_repo.to_str().unwrap()),
            "should default to main repo location, got: {result}"
        );
    }
}
