//! Git operations via CLI for file change tracking.
//!
//! All functions shell out to `git` via `tokio::process::Command`, which works
//! reliably in sandbox containers where git is always available.

use std::collections::{HashMap, HashSet};

use tokio::process::Command;
use tracing::warn;

/// Size limits matching the original TypeScript implementation.
const MAX_FILE_LINES: i64 = 10_000;

/// Errors from git operations.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("Not a git repository")]
    NotGitRepo,
    #[error("Git command failed: {0}")]
    CommandFailed(String),
    #[error("Git command timed out")]
    Timeout,
}

/// A file's git change information.
#[derive(Debug, Clone)]
pub struct GitFileInfo {
    pub status: String, // "added", "modified", "deleted", "renamed"
    pub additions: i32,
    pub deletions: i32,
    pub is_staged: bool,
}

/// Response from `get_git_diff_detailed`.
#[derive(Debug, Default)]
pub struct GitDiffResponse {
    pub files: Vec<GitDiffFile>,
    pub total_additions: i32,
    pub total_deletions: i32,
    pub compare_base: String,
}

/// A file in the git diff.
#[derive(Debug, Clone)]
pub struct GitDiffFile {
    pub status: String,
    pub filename: String,
    pub additions: i32,
    pub deletions: i32,
    pub is_staged: bool,
}

/// Per-file change info for `read_file_with_changes`.
#[derive(Debug, Default)]
pub struct FileChangeInfo {
    pub old_content: Option<String>,
    pub additions: i32,
    pub deletions: i32,
    pub is_modified: bool,
}

/// Compare reference used for git diffs.
struct CompareRef {
    reference: String,
}

/// A git repository discovered within the workspace.
#[derive(Debug, Clone)]
pub struct GitRepo {
    /// Absolute path to the repository root.
    pub abs_path: String,
    /// Path prefix relative to the workspace dir. Empty when the workspace root
    /// is itself a repo, otherwise the subdirectory name (e.g.
    /// `"moatless"`).
    pub prefix: String,
}

// ─── Helper ──────────────────────────────────────────────────────

/// What a git invocation said, whatever it exited with.
pub(super) struct GitOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl GitOutput {
    pub(super) fn succeeded(&self) -> bool {
        self.exit_code == 0
    }
}

/// Run a git command and keep its exit status.
///
/// A non-zero exit is an answer, not an error, for commands like `diff
/// --no-index` ("they differ") or `rev-parse` ("no such ref"); the caller reads
/// it from the output.
pub(super) async fn run_git_capture(
    workspace_dir: &str,
    args: &[&str],
    timeout_secs: u64,
) -> Result<GitOutput, GitError> {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        Command::new("git")
            .args(args)
            .current_dir(workspace_dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            // A held or kill-orphaned `index.lock` fails the agent's `git commit`.
            // Porcelain `git diff` ignores this and rewrites a stat-dirty index,
            // so diff the working tree with `diff-index`/`diff-files` instead.
            .env("GIT_OPTIONAL_LOCKS", "0")
            .kill_on_drop(true)
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => Ok(GitOutput {
            // A signalled git has no code; treat it as a failure that is not 0.
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        }),
        Ok(Err(e)) => Err(GitError::CommandFailed(format!("Failed to spawn git: {e}"))),
        Err(_) => Err(GitError::Timeout),
    }
}

async fn run_git(
    workspace_dir: &str,
    args: &[&str],
    timeout_secs: u64,
) -> Result<String, GitError> {
    let output = run_git_capture(workspace_dir, args, timeout_secs).await?;
    if output.succeeded() {
        Ok(output.stdout)
    } else {
        Err(GitError::CommandFailed(format!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            output.stderr.trim()
        )))
    }
}

/// Run a git command, returning empty string on failure (non-critical).
async fn run_git_quiet(workspace_dir: &str, args: &[&str], timeout_secs: u64) -> String {
    run_git(workspace_dir, args, timeout_secs)
        .await
        .unwrap_or_default()
}

// ─── Public API ──────────────────────────────────────────────────

/// Check if the workspace is a git repository.
pub async fn is_git_repository(workspace_dir: &str) -> bool {
    run_git(workspace_dir, &["rev-parse", "--git-dir"], 5)
        .await
        .is_ok()
}

/// Discover git repositories within the workspace directory.
///
/// Scans immediate non-hidden subdirectories (repos are always cloned one level
/// deep) and returns each git repo found, sorted by its directory-name prefix.
pub async fn discover_git_repos(workspace_dir: &str) -> Vec<GitRepo> {
    let mut entries = match tokio::fs::read_dir(workspace_dir).await {
        Ok(entries) => entries,
        Err(e) => {
            warn!(error = %e, dir = %workspace_dir, "Failed to scan workspace for git repos");
            return Vec::new();
        }
    };

    let mut repos = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let file_type = match entry.file_type().await {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !file_type.is_dir() {
            continue;
        }

        // Skip hidden dirs (covers `.git` and caches). Every other visible child
        // is probed directly, so a repo is never missed just because its
        // directory name resembles a build dir (e.g. `target`, `node_modules`).
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }

        let abs_path = entry.path().to_string_lossy().to_string();
        if is_git_repository(&abs_path).await {
            repos.push(GitRepo {
                abs_path,
                prefix: name,
            });
        }
    }

    repos.sort_by(|a, b| a.prefix.cmp(&b.prefix));
    repos
}

/// Resolve which discovered repo a workspace-relative path belongs to.
///
/// Returns the owning repo and the path relative to its root, picking the
/// longest-prefix match when repos are nested.
fn resolve_repo_for_path<'a>(
    repos: &'a [GitRepo],
    file_path: &str,
) -> Option<(&'a GitRepo, String)> {
    let mut best: Option<(&'a GitRepo, String)> = None;
    for repo in repos {
        let rel = if repo.prefix.is_empty() {
            Some(file_path.to_string())
        } else {
            file_path
                .strip_prefix(&format!("{}/", repo.prefix))
                .map(ToString::to_string)
        };

        if let Some(rel) = rel {
            let is_better = match &best {
                Some((current, _)) => repo.prefix.len() > current.prefix.len(),
                None => true,
            };
            if is_better {
                best = Some((repo, rel));
            }
        }
    }
    best
}

/// Detect base branch (main or master).
async fn detect_base_branch(workspace_dir: &str) -> String {
    if run_git(workspace_dir, &["rev-parse", "--verify", "main"], 5)
        .await
        .is_ok()
    {
        return "main".to_string();
    }
    if run_git(workspace_dir, &["rev-parse", "--verify", "master"], 5)
        .await
        .is_ok()
    {
        return "master".to_string();
    }
    "main".to_string()
}

/// Find the best compare reference for diffs.
async fn get_best_compare_reference(workspace_dir: &str) -> CompareRef {
    let base = detect_base_branch(workspace_dir).await;
    let origin_branch = format!("origin/{base}");

    if run_git(workspace_dir, &["rev-parse", "--verify", &origin_branch], 5)
        .await
        .is_ok()
    {
        return CompareRef {
            reference: origin_branch,
        };
    }

    let current = run_git_quiet(workspace_dir, &["branch", "--show-current"], 5).await;
    let current = current.trim();
    if !current.is_empty() {
        if let Ok(output) = run_git(workspace_dir, &["merge-base", &base, current], 5).await {
            let merge_base = output.trim().to_string();
            if !merge_base.is_empty() {
                return CompareRef {
                    reference: merge_base,
                };
            }
        }
    }

    CompareRef { reference: base }
}

/// Parse `git status --porcelain` output.
async fn get_git_status(
    workspace_dir: &str,
) -> Result<(Vec<String>, Vec<String>, Vec<String>), GitError> {
    let output = run_git(workspace_dir, &["status", "--porcelain"], 5).await?;
    let mut staged = Vec::new();
    let mut modified = Vec::new();
    let mut untracked = Vec::new();

    for line in output.lines() {
        if line.len() < 3 {
            continue;
        }
        let status = &line[..2];
        let filename = line[3..].trim().to_string();

        let index_status = status.as_bytes().first().copied().unwrap_or(b' ');
        let worktree_status = status.as_bytes().get(1).copied().unwrap_or(b' ');

        if index_status != b' ' && index_status != b'?' {
            staged.push(filename.clone());
        }
        if worktree_status == b'M' || worktree_status == b'D' {
            modified.push(filename.clone());
        }
        if status == "??" {
            untracked.push(filename);
        }
    }

    Ok((staged, modified, untracked))
}

/// Get numstat data for a compare reference.
async fn get_numstat(workspace_dir: &str, compare_base: &str) -> HashMap<String, (i32, i32, bool)> {
    let output = run_git_quiet(
        workspace_dir,
        &["diff-index", "-M", "--numstat", compare_base],
        10,
    )
    .await;

    let mut map = HashMap::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let is_binary = parts[0] == "-" && parts[1] == "-";
        let additions = if is_binary {
            0
        } else {
            parts[0].parse::<i32>().unwrap_or(0)
        };
        let deletions = if is_binary {
            0
        } else {
            parts[1].parse::<i32>().unwrap_or(0)
        };
        map.insert(parts[2].to_string(), (additions, deletions, is_binary));
    }
    map
}

/// Get the diff string + compare base.
async fn get_diff_string(workspace_dir: &str) -> (String, String) {
    let compare_ref = get_best_compare_reference(workspace_dir).await;
    let compare_base = compare_ref.reference;

    let diff_output = run_git_quiet(
        workspace_dir,
        &["diff-index", "-M", "--patch", &compare_base],
        30,
    )
    .await;

    let untracked_diff = get_untracked_files_diff(workspace_dir).await;
    let combined = if untracked_diff.is_empty() {
        diff_output
    } else {
        format!("{diff_output}\n{untracked_diff}")
    };

    (combined, compare_base)
}

/// Build diff content for untracked files.
async fn get_untracked_files_diff(workspace_dir: &str) -> String {
    let output = run_git_quiet(
        workspace_dir,
        &["ls-files", "--others", "--exclude-standard"],
        10,
    )
    .await;

    use std::fmt::Write;

    let mut diff = String::new();
    for filename in output.lines().filter(|l| !l.trim().is_empty()) {
        let file_path = std::path::Path::new(workspace_dir).join(filename);
        if let Ok(content) = tokio::fs::read_to_string(&file_path).await {
            let _ = writeln!(diff, "diff --git a/{filename} b/{filename}");
            diff.push_str("new file mode 100644\n");
            diff.push_str("index 0000000..0000000\n");
            diff.push_str("--- /dev/null\n");
            let _ = writeln!(diff, "+++ b/{filename}");
            let lines: Vec<&str> = content.lines().collect();
            if !lines.is_empty() {
                let _ = writeln!(diff, "@@ -0,0 +1,{} @@", lines.len());
                for line in lines {
                    let _ = writeln!(diff, "+{line}");
                }
            }
        } else {
            let _ = writeln!(diff, "diff --git a/{filename} b/{filename}");
            diff.push_str("new file mode 100644\n");
            diff.push_str("Binary file\n");
        }
    }
    diff
}

/// Parse diff chunks to extract file statuses and change counts.
fn parse_diff_files(
    diff_string: &str,
    staged_set: &HashSet<String>,
    numstat: &HashMap<String, (i32, i32, bool)>,
) -> Vec<GitDiffFile> {
    let chunks: Vec<&str> = diff_string.split("\ndiff --git ").collect();
    let mut files = Vec::new();

    for chunk in chunks {
        if chunk.trim().is_empty() {
            continue;
        }

        // Extract filename from "a/file b/file" pattern
        let first_line = chunk.lines().next().unwrap_or("");
        let filename = if let Some(caps) = extract_filename(first_line) {
            caps
        } else {
            continue;
        };

        if let Some(&(add, del, is_binary)) = numstat.get(&filename) {
            if is_binary || (add + del) as i64 > MAX_FILE_LINES {
                continue;
            }
        }

        let status = determine_file_status(chunk);

        let (additions, deletions) = count_changes(chunk);

        let is_staged = staged_set.contains(&filename);

        files.push(GitDiffFile {
            status,
            filename,
            additions,
            deletions,
            is_staged,
        });
    }

    files
}

fn extract_filename(first_line: &str) -> Option<String> {
    // Pattern: "a/path b/path" or full "diff --git a/path b/path"
    let line = first_line.strip_prefix("diff --git ").unwrap_or(first_line);
    let parts: Vec<&str> = line.splitn(2, " b/").collect();
    if parts.len() == 2 {
        Some(parts[1].to_string())
    } else {
        None
    }
}

fn determine_file_status(chunk: &str) -> String {
    for line in chunk.lines().take(5) {
        if line.contains("new file mode") {
            return "added".to_string();
        }
        if line.contains("deleted file mode") {
            return "deleted".to_string();
        }
        if line.contains("rename from") || line.contains("rename to") {
            return "renamed".to_string();
        }
    }
    "modified".to_string()
}

fn count_changes(chunk: &str) -> (i32, i32) {
    let mut additions = 0;
    let mut deletions = 0;
    for line in chunk.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            additions += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
        }
    }
    (additions, deletions)
}

/// Get detailed git diff information for the workspace.
///
/// Diffs are computed per-repo and merged, with each file's path prefixed by
/// its repo's workspace-relative location to match the file tree. Empty when no
/// repos are found.
pub async fn get_git_diff_detailed(workspace_dir: &str) -> GitDiffResponse {
    let repos = discover_git_repos(workspace_dir).await;
    if repos.is_empty() {
        return GitDiffResponse::default();
    }

    let mut combined = GitDiffResponse::default();
    for repo in &repos {
        let mut resp = get_repo_diff_detailed(&repo.abs_path).await;

        // Prefix filenames with the repo's workspace-relative path so they
        // match the tree paths built from the workspace root.
        if !repo.prefix.is_empty() {
            for file in &mut resp.files {
                file.filename = format!("{}/{}", repo.prefix, file.filename);
            }
        }

        combined.files.append(&mut resp.files);
        combined.total_additions += resp.total_additions;
        combined.total_deletions += resp.total_deletions;
        if combined.compare_base.is_empty() {
            combined.compare_base = resp.compare_base;
        }
    }

    combined
}

/// Get detailed git diff information for a single repository root.
async fn get_repo_diff_detailed(workspace_dir: &str) -> GitDiffResponse {
    let (diff_string, compare_base) = get_diff_string(workspace_dir).await;
    let (staged, _modified, untracked) = match get_git_status(workspace_dir).await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "Failed to get git status");
            return GitDiffResponse {
                compare_base,
                ..Default::default()
            };
        }
    };

    let staged_set: HashSet<String> = staged.into_iter().collect();
    let numstat = get_numstat(workspace_dir, &compare_base).await;

    let mut files = parse_diff_files(&diff_string, &staged_set, &numstat);

    let existing_filenames: HashSet<String> = files.iter().map(|f| f.filename.clone()).collect();
    for filename in &untracked {
        if existing_filenames.contains(filename.as_str()) {
            continue;
        }
        let file_path = std::path::Path::new(workspace_dir).join(filename);
        if let Ok(content) = tokio::fs::read_to_string(&file_path).await {
            let line_count = content.lines().count();
            files.push(GitDiffFile {
                status: "added".to_string(),
                filename: filename.clone(),
                additions: line_count as i32,
                deletions: 0,
                is_staged: false,
            });
        }
    }

    let total_additions: i32 = files.iter().map(|f| f.additions).sum();
    let total_deletions: i32 = files.iter().map(|f| f.deletions).sum();

    GitDiffResponse {
        files,
        total_additions,
        total_deletions,
        compare_base,
    }
}

/// Get change info for a specific file (used by read-file-with-changes).
///
/// `file_path` is workspace-root-relative; it is resolved to the git repo that
/// owns it and the diff is computed relative to that repo.
pub async fn get_file_change_info(workspace_dir: &str, file_path: &str) -> FileChangeInfo {
    let repos = discover_git_repos(workspace_dir).await;
    let Some((repo, file_path)) = resolve_repo_for_path(&repos, file_path) else {
        return FileChangeInfo::default();
    };
    let workspace_dir = repo.abs_path.as_str();
    let file_path = file_path.as_str();

    let compare_ref = get_best_compare_reference(workspace_dir).await;
    let compare_base = &compare_ref.reference;

    let numstat_output = run_git_quiet(
        workspace_dir,
        &[
            "diff-index",
            "-M",
            "--numstat",
            compare_base,
            "--",
            file_path,
        ],
        10,
    )
    .await;

    if numstat_output.trim().is_empty() {
        return FileChangeInfo::default();
    }

    let parts: Vec<&str> = numstat_output.trim().split('\t').collect();
    let additions = parts
        .first()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);
    let deletions = parts
        .get(1)
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);

    let status_output = run_git_quiet(
        workspace_dir,
        &[
            "diff-index",
            "-M",
            "--name-status",
            compare_base,
            "--",
            file_path,
        ],
        10,
    )
    .await;

    let status_code = status_output
        .trim()
        .split('\t')
        .next()
        .unwrap_or("")
        .to_string();

    let old_content = if status_code.starts_with('M') || status_code.starts_with('D') {
        let show_ref = format!("{compare_base}:{file_path}");
        run_git(workspace_dir, &["show", &show_ref], 5).await.ok()
    } else {
        None
    };

    FileChangeInfo {
        old_content,
        additions,
        deletions,
        is_modified: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_filename_from_diff_header() {
        assert_eq!(
            extract_filename("a/src/main.rs b/src/main.rs"),
            Some("src/main.rs".to_string())
        );
        assert_eq!(
            extract_filename("diff --git a/foo.txt b/foo.txt"),
            Some("foo.txt".to_string())
        );
    }

    #[test]
    fn determine_status_new_file() {
        let chunk = "a/new.rs b/new.rs\nnew file mode 100644\nindex 000..111\n";
        assert_eq!(determine_file_status(chunk), "added");
    }

    #[test]
    fn determine_status_deleted_file() {
        let chunk = "a/old.rs b/old.rs\ndeleted file mode 100644\n";
        assert_eq!(determine_file_status(chunk), "deleted");
    }

    #[test]
    fn determine_status_modified_file() {
        let chunk = "a/mod.rs b/mod.rs\nindex abc..def\n--- a/mod.rs\n+++ b/mod.rs\n";
        assert_eq!(determine_file_status(chunk), "modified");
    }

    #[test]
    fn count_changes_basic() {
        let chunk =
            "header\n--- a/f\n+++ b/f\n+added line\n-removed line\n context\n+another add\n";
        let (add, del) = count_changes(chunk);
        assert_eq!(add, 2);
        assert_eq!(del, 1);
    }

    fn repo(prefix: &str) -> GitRepo {
        GitRepo {
            abs_path: format!("/ws/{prefix}"),
            prefix: prefix.to_string(),
        }
    }

    #[test]
    fn resolve_repo_root_repo() {
        let repos = vec![GitRepo {
            abs_path: "/ws".to_string(),
            prefix: String::new(),
        }];
        let (r, rel) = resolve_repo_for_path(&repos, "src/main.rs").unwrap();
        assert_eq!(r.prefix, "");
        assert_eq!(rel, "src/main.rs");
    }

    #[test]
    fn resolve_repo_by_prefix() {
        let repos = vec![repo("app"), repo("lib")];
        let (r, rel) = resolve_repo_for_path(&repos, "lib/src/x.rs").unwrap();
        assert_eq!(r.prefix, "lib");
        assert_eq!(rel, "src/x.rs");
    }

    #[test]
    fn resolve_repo_prefers_longest_prefix() {
        let repos = vec![repo("app"), repo("app/nested")];
        let (r, rel) = resolve_repo_for_path(&repos, "app/nested/src/x.rs").unwrap();
        assert_eq!(r.prefix, "app/nested");
        assert_eq!(rel, "src/x.rs");
    }

    #[test]
    fn resolve_repo_no_match() {
        let repos = vec![repo("app")];
        assert!(resolve_repo_for_path(&repos, "other/x.rs").is_none());
    }

    async fn git_init(dir: &std::path::Path) {
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t.co"],
            vec!["config", "user.name", "t"],
        ] {
            Command::new("git")
                .args(&args)
                .current_dir(dir)
                .output()
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn discover_repos_scans_first_level_only() {
        // A repo at the workspace root itself is not discovered — repos are
        // always cloned into first-level subdirectories, so only those are
        // scanned.
        let tmp = tempfile::TempDir::new().unwrap();
        git_init(tmp.path()).await;

        let repos = discover_git_repos(tmp.path().to_str().unwrap()).await;
        assert!(repos.is_empty());
    }

    #[tokio::test]
    async fn discover_repos_in_subdirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        // Workspace root is NOT a repo; some subdirs are, one is not. A repo
        // whose dir name resembles a build dir (`target`) must still be found,
        // while hidden dirs (`.hidden`) are skipped.
        std::fs::create_dir(root.join("repo-a")).unwrap();
        std::fs::create_dir(root.join("repo-b")).unwrap();
        std::fs::create_dir(root.join("target")).unwrap();
        std::fs::create_dir(root.join("plain")).unwrap();
        std::fs::create_dir(root.join(".hidden")).unwrap();
        git_init(&root.join("repo-a")).await;
        git_init(&root.join("repo-b")).await;
        git_init(&root.join("target")).await;
        git_init(&root.join(".hidden")).await;

        let repos = discover_git_repos(root.to_str().unwrap()).await;
        let prefixes: Vec<&str> = repos.iter().map(|r| r.prefix.as_str()).collect();
        assert_eq!(prefixes, vec!["repo-a", "repo-b", "target"]);
    }

    async fn git_commit_all(dir: &std::path::Path, msg: &str) {
        for args in [vec!["add", "-A"], vec!["commit", "-q", "-m", msg]] {
            Command::new("git")
                .args(&args)
                .current_dir(dir)
                .output()
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn diff_detailed_prefixes_subdir_repo_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let repo_dir = root.join("my-repo");
        std::fs::create_dir(&repo_dir).unwrap();
        git_init(&repo_dir).await;

        std::fs::write(repo_dir.join("tracked.txt"), "one\n").unwrap();
        git_commit_all(&repo_dir, "init").await;
        std::fs::write(repo_dir.join("tracked.txt"), "one\ntwo\n").unwrap();
        std::fs::write(repo_dir.join("new.txt"), "brand new\n").unwrap();

        let resp = get_git_diff_detailed(root.to_str().unwrap()).await;
        let names: HashSet<&str> = resp.files.iter().map(|f| f.filename.as_str()).collect();

        // Filenames are prefixed with the repo's workspace-relative directory so
        // they line up with the file tree rooted at the workspace.
        assert!(
            names.contains("my-repo/tracked.txt"),
            "expected prefixed tracked file, got {names:?}"
        );
        assert!(
            names.contains("my-repo/new.txt"),
            "expected prefixed untracked file, got {names:?}"
        );

        let info = get_file_change_info(root.to_str().unwrap(), "my-repo/tracked.txt").await;
        assert!(info.is_modified);
        assert_eq!(info.additions, 1);
    }
}
