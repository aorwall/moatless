//! The git review surfaces: the patch a reviewer reads, the branch state
//! behind it, and the refs they can diff against.
//!
//! # What a path in here means
//!
//! Every path a patch or a status entry carries is **workspace-relative**, the
//! same space [`super::tree`] and the read APIs use, so a file named by a patch
//! is a file the caller can turn round and open. A workspace holds 0..n
//! repositories, each cloned into its own subdirectory, so a patch spans all of
//! them and git is asked to write each repository's own prefix into the patch
//! headers (`--src-prefix`/`--dst-prefix`).
//!
//! Renames are asked for as a delete plus an add: git applies those prefixes to
//! the `diff --git` header but not to `rename from`/`rename to`, and a rename
//! header naming a repository-relative path is a path the caller cannot open.
//!
//! # What answers for the primary repository alone
//!
//! Branch state and refs. "Which branch is this, how far ahead of its upstream,
//! what can I diff against" are questions about one repository's HEAD and one
//! repository's ref namespace; merging several repositories' answers would
//! invent a branch that does not exist. The diff spans the workspace because a
//! reviewer reads changes, not repositories.

use std::collections::HashMap;

use tracing::warn;

use super::git::{self, GitError, GitRepo};

/// The most one scope's patch carries, matching the reference client's own
/// server — which is where the client's expectations about a patch's size, and
/// its willingness to render one, were formed.
const WORKING_TREE_PATCH_MAX_BYTES: usize = 120_000;
const UNTRACKED_PATCH_MAX_BYTES: usize = 80_000;
const BRANCH_PATCH_MAX_BYTES: usize = 120_000;

/// What a cut patch ends with, matching the reference server's marker so a
/// client that looks for it finds it.
const TRUNCATION_MARKER: &str = "\n\n[truncated]";

/// The most refs one listing carries. A repository past this has more branches
/// than any picker is usable with, and the caller is told the list was cut.
const MAX_REFS: usize = 2_000;

/// Longest a single git invocation here may take. Diffs read the whole working
/// tree, so they are given the same 30s the file-tree overlay already uses;
/// everything else is a metadata read.
const DIFF_TIMEOUT_SECS: u64 = 30;
const METADATA_TIMEOUT_SECS: u64 = 5;

/// Arguments every patch here is produced with.
///
/// `--no-color`/`--no-ext-diff`/`--no-textconv` keep a repository's own git
/// configuration from rewriting the patch into something that is no longer a
/// patch, and `core.quotepath=false` keeps a non-ASCII path readable rather
/// than octal-escaped.
const PATCH_ARGS: &[&str] = &[
    "-c",
    "core.quotepath=false",
    "diff",
    "--patch",
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "--no-renames",
    "--minimal",
];

/// Both diff scopes of one workspace, and what each repository was compared
/// against.
#[derive(Debug, Default)]
pub struct DiffPreview {
    pub working_tree: Patch,
    pub branch: Patch,
    pub ranges: Vec<RepoRange>,
}

/// One scope's patch, and whether it is the whole of one.
#[derive(Debug, Default)]
pub struct Patch {
    pub diff: String,
    pub truncated: bool,
}

/// What one repository in the workspace was compared against.
#[derive(Debug)]
pub struct RepoRange {
    /// Workspace-relative repository directory; empty at the workspace root.
    pub prefix: String,
    /// The resolved base, or `None` when the repository has none to compare to.
    pub base_ref: Option<String>,
    /// The current branch, or `None` when detached or unborn.
    pub head_ref: Option<String>,
}

/// One repository's branch state.
#[derive(Debug, Default)]
pub struct RepoStatus {
    pub is_repo: bool,
    pub ref_name: Option<String>,
    pub is_default_ref: bool,
    pub has_primary_remote: bool,
    pub has_upstream: bool,
    pub ahead_count: u32,
    pub behind_count: u32,
    pub has_working_tree_changes: bool,
    pub working_tree_files: Vec<WorkingTreeFile>,
}

/// One file's uncommitted line counts, at a workspace-relative path.
#[derive(Debug)]
pub struct WorkingTreeFile {
    pub path: String,
    pub insertions: u32,
    pub deletions: u32,
}

/// One repository's refs, as a picker offers them.
#[derive(Debug, Default)]
pub struct RefListing {
    pub is_repo: bool,
    pub has_primary_remote: bool,
    pub refs: Vec<RefInfo>,
    pub truncated: bool,
}

/// One ref a diff can be taken against.
#[derive(Debug)]
pub struct RefInfo {
    /// `main` for a local ref, `origin/main` for a remote-tracking one.
    pub name: String,
    pub is_remote: bool,
    pub remote_name: Option<String>,
    pub current: bool,
    pub is_default: bool,
}

// ─── Diff preview ────────────────────────────────────────────────

/// Both diff scopes of a workspace: what is not committed, and what this branch
/// has committed since it left its base.
///
/// `base_ref` overrides each repository's detected base. A repository that
/// cannot resolve it contributes nothing to the branch scope rather than
/// silently falling back — a patch taken against a base the caller did not ask
/// for reads as the one they did.
pub async fn diff_preview(
    workspace_dir: &str,
    base_ref: Option<&str>,
    ignore_whitespace: bool,
) -> DiffPreview {
    let repos = git::discover_git_repos(workspace_dir).await;
    let mut working_tree = PatchBuilder::new(WORKING_TREE_PATCH_MAX_BYTES);
    let mut branch = PatchBuilder::new(BRANCH_PATCH_MAX_BYTES);
    let mut ranges = Vec::with_capacity(repos.len());

    for repo in &repos {
        let prefixes = prefix_args(&repo.prefix);
        working_tree.push(&working_tree_patch(repo, &prefixes, ignore_whitespace).await);

        let head_ref = current_branch(&repo.abs_path).await;
        let resolved_base = match base_ref {
            Some(requested) => verified_ref(&repo.abs_path, requested).await,
            None => detect_base_ref(&repo.abs_path, head_ref.as_deref()).await,
        };
        if let Some(base) = &resolved_base {
            branch.push(&branch_patch(repo, &prefixes, base, ignore_whitespace).await);
        }

        ranges.push(RepoRange {
            prefix: repo.prefix.clone(),
            base_ref: resolved_base,
            head_ref,
        });
    }

    DiffPreview {
        working_tree: working_tree.finish(),
        branch: branch.finish(),
        ranges,
    }
}

/// One repository's uncommitted changes: tracked against HEAD, plus every
/// untracked file as a whole-file addition.
async fn working_tree_patch(repo: &GitRepo, prefixes: &[String], ignore_whitespace: bool) -> Patch {
    let mut args: Vec<&str> = PATCH_ARGS.to_vec();
    args.extend(prefixes.iter().map(String::as_str));
    if ignore_whitespace {
        args.push("--ignore-all-space");
    }
    args.push("HEAD");
    args.push("--");

    let mut patch = PatchBuilder::new(WORKING_TREE_PATCH_MAX_BYTES);
    // A repository whose HEAD is unborn has nothing to diff against and every
    // file in it is untracked, which the pass below covers on its own.
    patch.push(&Patch {
        diff: quiet(&repo.abs_path, &args, DIFF_TIMEOUT_SECS).await,
        truncated: false,
    });
    patch.push(&untracked_patch(repo, prefixes).await);
    patch.finish()
}

/// Every untracked file in one repository, as an addition diff.
///
/// `diff --no-index` against `/dev/null` is what makes these real patches
/// rather than hand-assembled ones: git decides the mode, the blob hashes,
/// whether the file is binary, and whether it ends in a newline.
async fn untracked_patch(repo: &GitRepo, prefixes: &[String]) -> Patch {
    let listed = quiet(
        &repo.abs_path,
        &[
            "-c",
            "core.quotepath=false",
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
        ],
        METADATA_TIMEOUT_SECS,
    )
    .await;

    let mut patch = PatchBuilder::new(UNTRACKED_PATCH_MAX_BYTES);
    for path in listed.split('\0').filter(|path| !path.is_empty()) {
        if patch.is_full() {
            break;
        }
        let mut args: Vec<&str> = PATCH_ARGS.to_vec();
        args.extend(prefixes.iter().map(String::as_str));
        args.extend(["--no-index", "--", "/dev/null", path]);
        // `--no-index` exits 1 exactly when the two sides differ, which for a
        // file against `/dev/null` is every time it produced a patch at all.
        patch.push(&Patch {
            diff: capture(&repo.abs_path, &args, DIFF_TIMEOUT_SECS)
                .await
                .map(|output| output.stdout)
                .unwrap_or_default(),
            truncated: false,
        });
    }
    patch.finish()
}

/// One repository's committed changes since it left `base_ref`.
async fn branch_patch(
    repo: &GitRepo,
    prefixes: &[String],
    base_ref: &str,
    ignore_whitespace: bool,
) -> Patch {
    let range = format!("{base_ref}...HEAD");
    let mut args: Vec<&str> = PATCH_ARGS.to_vec();
    args.extend(prefixes.iter().map(String::as_str));
    if ignore_whitespace {
        args.push("--ignore-all-space");
    }
    args.push(&range);

    Patch {
        diff: quiet(&repo.abs_path, &args, DIFF_TIMEOUT_SECS).await,
        truncated: false,
    }
}

/// The `--src-prefix`/`--dst-prefix` pair that writes a repository's
/// workspace-relative directory into its patch headers.
fn prefix_args(repo_prefix: &str) -> Vec<String> {
    let path = if repo_prefix.is_empty() {
        String::new()
    } else {
        format!("{repo_prefix}/")
    };
    vec![
        format!("--src-prefix=a/{path}"),
        format!("--dst-prefix=b/{path}"),
    ]
}

// ─── Branch state ────────────────────────────────────────────────

/// The primary repository's branch state, with working-tree paths lifted into
/// the workspace's path space.
pub async fn status(workspace_dir: &str, repo_dir: &str) -> RepoStatus {
    let porcelain = match capture(
        repo_dir,
        &["status", "--porcelain=2", "--branch"],
        METADATA_TIMEOUT_SECS,
    )
    .await
    {
        Ok(output) if output.succeeded() => output.stdout,
        // Not a repository, or a git that could not answer: both leave the
        // caller with "there is nothing to review here", which is true.
        _ => return RepoStatus::default(),
    };

    let mut status = RepoStatus {
        is_repo: true,
        has_primary_remote: has_primary_remote(repo_dir).await,
        ..RepoStatus::default()
    };
    for line in porcelain.lines() {
        let Some(header) = line.strip_prefix("# ") else {
            // Every other line is one changed path — tracked, untracked or
            // unmerged. Which it is only matters to the counts below.
            status.has_working_tree_changes = true;
            continue;
        };
        if let Some(head) = header.strip_prefix("branch.head ") {
            status.ref_name = (head != "(detached)").then(|| head.to_string());
        } else if header.starts_with("branch.upstream ") {
            status.has_upstream = true;
        } else if let Some(ab) = header.strip_prefix("branch.ab ") {
            (status.ahead_count, status.behind_count) = parse_ahead_behind(ab);
        }
    }

    let default_branch = default_branch(repo_dir).await;
    status.is_default_ref = match (&status.ref_name, &default_branch) {
        (Some(head), Some(default)) => head == default,
        _ => false,
    };
    status.working_tree_files = working_tree_files(workspace_dir, repo_dir).await;
    status
}

/// `+2 -3` as the pair a client counts with.
fn parse_ahead_behind(field: &str) -> (u32, u32) {
    let mut counts = field.split_whitespace();
    let ahead = counts
        .next()
        .and_then(|value| value.strip_prefix('+'))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let behind = counts
        .next()
        .and_then(|value| value.strip_prefix('-'))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    (ahead, behind)
}

/// Per-file line counts for what is not committed, at workspace-relative paths.
///
/// An unborn HEAD has nothing to diff against, so the staged and unstaged
/// counts are read separately and added — which is the same total the single
/// `diff HEAD` gives once a first commit exists.
async fn working_tree_files(workspace_dir: &str, repo_dir: &str) -> Vec<WorkingTreeFile> {
    let numstat = capture(
        repo_dir,
        &["-c", "core.quotepath=false", "diff", "HEAD", "--numstat"],
        METADATA_TIMEOUT_SECS,
    )
    .await;
    let mut totals: HashMap<String, (u32, u32)> = HashMap::new();
    match numstat {
        Ok(output) if output.succeeded() => accumulate_numstat(&output.stdout, &mut totals),
        _ => {
            for args in [
                ["-c", "core.quotepath=false", "diff", "--numstat"].as_slice(),
                [
                    "-c",
                    "core.quotepath=false",
                    "diff",
                    "--cached",
                    "--numstat",
                ]
                .as_slice(),
            ] {
                accumulate_numstat(
                    &quiet(repo_dir, args, METADATA_TIMEOUT_SECS).await,
                    &mut totals,
                );
            }
        }
    }

    let prefix = workspace_relative_prefix(workspace_dir, repo_dir);
    let mut files: Vec<WorkingTreeFile> = totals
        .into_iter()
        .map(|(path, (insertions, deletions))| WorkingTreeFile {
            path: format!("{prefix}{path}"),
            insertions,
            deletions,
        })
        .collect();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
}

/// Add one `--numstat` block's counts into `totals`.
///
/// A binary file reports `-` for both counts; it is a changed file with no
/// lines, so it lands with zeroes rather than being dropped.
fn accumulate_numstat(output: &str, totals: &mut HashMap<String, (u32, u32)>) {
    for line in output.lines() {
        let mut fields = line.split('\t');
        let (Some(insertions), Some(deletions), Some(path)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        let entry = totals.entry(path.to_string()).or_default();
        entry.0 += insertions.parse().unwrap_or(0);
        entry.1 += deletions.parse().unwrap_or(0);
    }
}

// ─── Refs ────────────────────────────────────────────────────────

/// Every branch of the primary repository a diff can be taken against.
pub async fn list_refs(repo_dir: &str) -> RefListing {
    let listed = match capture(
        repo_dir,
        &[
            "for-each-ref",
            "--format=%(refname)%09%(refname:short)%09%(HEAD)",
            "refs/heads",
            "refs/remotes",
        ],
        METADATA_TIMEOUT_SECS,
    )
    .await
    {
        Ok(output) if output.succeeded() => output.stdout,
        _ => return RefListing::default(),
    };

    let default_branch = default_branch(repo_dir).await;
    let mut refs = Vec::new();
    let mut truncated = false;
    for line in listed.lines() {
        let mut fields = line.split('\t');
        let (Some(full_name), Some(short_name)) = (fields.next(), fields.next()) else {
            continue;
        };
        // `origin/HEAD` is a symbolic ref onto another entry in this same
        // listing, so offering it would put the default branch in the picker
        // twice under two names.
        if full_name.ends_with("/HEAD") {
            continue;
        }
        if refs.len() >= MAX_REFS {
            truncated = true;
            break;
        }
        let remote_name = full_name
            .strip_prefix("refs/remotes/")
            .and_then(|rest| rest.split_once('/'))
            .map(|(remote, _)| remote.to_string());
        let branch_name = match &remote_name {
            Some(remote) => short_name
                .strip_prefix(&format!("{remote}/"))
                .unwrap_or(short_name),
            None => short_name,
        };
        refs.push(RefInfo {
            name: short_name.to_string(),
            is_remote: remote_name.is_some(),
            remote_name,
            current: fields.next() == Some("*"),
            is_default: default_branch.as_deref() == Some(branch_name),
        });
    }

    RefListing {
        is_repo: true,
        has_primary_remote: has_primary_remote(repo_dir).await,
        refs,
        truncated,
    }
}

// ─── Repository facts ────────────────────────────────────────────

/// The branch HEAD is on, or `None` when detached or unborn.
async fn current_branch(repo_dir: &str) -> Option<String> {
    let branch = quiet(
        repo_dir,
        &["branch", "--show-current"],
        METADATA_TIMEOUT_SECS,
    )
    .await
    .trim()
    .to_string();
    (!branch.is_empty()).then_some(branch)
}

/// Whether the repository has an `origin`.
async fn has_primary_remote(repo_dir: &str) -> bool {
    quiet(repo_dir, &["remote"], METADATA_TIMEOUT_SECS)
        .await
        .lines()
        .any(|remote| remote.trim() == "origin")
}

/// The repository's default branch, by short name.
///
/// `origin/HEAD` is what the remote itself says, so it is asked first; the
/// conventional names are only a fallback for a clone that never recorded one.
async fn default_branch(repo_dir: &str) -> Option<String> {
    let symbolic = quiet(
        repo_dir,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        METADATA_TIMEOUT_SECS,
    )
    .await;
    if let Some(branch) = symbolic.trim().strip_prefix("origin/") {
        if !branch.is_empty() {
            return Some(branch.to_string());
        }
    }
    for candidate in ["main", "master"] {
        if verified_ref(repo_dir, candidate).await.is_some() {
            return Some(candidate.to_string());
        }
    }
    None
}

/// What this repository's branch should be read against.
///
/// The remote-tracking ref is preferred over the local branch of the same name:
/// a sandbox checks out a task branch and the local `main` beside it is
/// whatever the clone happened to fetch, while `origin/main` is what the change
/// will actually be proposed against.
async fn detect_base_ref(repo_dir: &str, head_ref: Option<&str>) -> Option<String> {
    let default = default_branch(repo_dir).await?;
    // A branch is not a base for itself; the working-tree scope is what has
    // anything to say about a repository sitting on its own default branch.
    if head_ref == Some(default.as_str()) {
        return None;
    }
    let remote = format!("origin/{default}");
    match verified_ref(repo_dir, &remote).await {
        Some(reference) => Some(reference),
        None => verified_ref(repo_dir, &default).await,
    }
}

/// `reference` if this repository resolves it, `None` otherwise.
async fn verified_ref(repo_dir: &str, reference: &str) -> Option<String> {
    let resolved = capture(
        repo_dir,
        &["rev-parse", "--verify", "--quiet", reference],
        METADATA_TIMEOUT_SECS,
    )
    .await
    .ok()?;
    resolved
        .succeeded()
        .then(|| reference.to_string())
        .filter(|_| !resolved.stdout.trim().is_empty())
}

/// `repo_dir` as a path prefix in the workspace's path space, with its trailing
/// separator. Empty when the repository *is* the workspace root.
fn workspace_relative_prefix(workspace_dir: &str, repo_dir: &str) -> String {
    let workspace = workspace_dir.trim_end_matches('/');
    let repo = repo_dir.trim_end_matches('/');
    match repo
        .strip_prefix(workspace)
        .map(|rest| rest.trim_matches('/'))
    {
        Some("") | None => String::new(),
        Some(rest) => format!("{rest}/"),
    }
}

// ─── git invocation ──────────────────────────────────────────────

async fn capture(dir: &str, args: &[&str], timeout_secs: u64) -> Result<git::GitOutput, GitError> {
    git::run_git_capture(dir, args, timeout_secs).await
}

/// Run a git command whose failure is not worth failing the whole surface for.
///
/// Every caller of this is assembling one part of a review that is still
/// readable without it, so a repository git refused to answer about contributes
/// nothing rather than emptying the panel.
async fn quiet(dir: &str, args: &[&str], timeout_secs: u64) -> String {
    match capture(dir, args, timeout_secs).await {
        Ok(output) if output.succeeded() => output.stdout,
        Ok(output) => {
            warn!(
                dir = %dir,
                exit_code = output.exit_code,
                stderr = %output.stderr.trim(),
                "git command for the review surface failed"
            );
            String::new()
        }
        Err(error) => {
            warn!(dir = %dir, error = %error, "git command for the review surface did not run");
            String::new()
        }
    }
}

// ─── Patch assembly ──────────────────────────────────────────────

/// A patch under construction, and the byte budget it stops at.
///
/// The budget is the whole scope's, not one repository's: a workspace whose
/// first repository has rewritten everything must not push the second one's
/// changes past what the client will render without saying so.
struct PatchBuilder {
    budget: usize,
    out: String,
    truncated: bool,
}

impl PatchBuilder {
    fn new(budget: usize) -> Self {
        Self {
            budget,
            out: String::new(),
            truncated: false,
        }
    }

    fn is_full(&self) -> bool {
        self.truncated
    }

    fn push(&mut self, patch: &Patch) {
        let chunk = patch.diff.trim_end();
        if !self.truncated && !chunk.is_empty() {
            let separator = if self.out.is_empty() { "" } else { "\n" };
            let remaining = self.budget.saturating_sub(self.out.len() + separator.len());
            self.out.push_str(separator);
            if chunk.len() <= remaining {
                self.out.push_str(chunk);
            } else {
                self.out.push_str(&chunk[..char_boundary(chunk, remaining)]);
                self.truncated = true;
            }
        }
        // A patch that was already cut short leaves the scope cut short, but
        // only after what it did carry is in.
        self.truncated |= patch.truncated;
    }

    fn finish(mut self) -> Patch {
        if self.truncated && !self.out.is_empty() && !self.out.ends_with(TRUNCATION_MARKER) {
            self.out.push_str(TRUNCATION_MARKER);
        }
        Patch {
            diff: self.out,
            truncated: self.truncated,
        }
    }
}

/// The largest offset at or below `max` that does not land inside a character.
fn char_boundary(text: &str, max: usize) -> usize {
    let mut end = max.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repository_subdirectory_becomes_the_patch_path_prefix() {
        assert_eq!(
            prefix_args("moatless"),
            vec![
                "--src-prefix=a/moatless/".to_string(),
                "--dst-prefix=b/moatless/".to_string()
            ]
        );
        assert_eq!(
            prefix_args(""),
            vec!["--src-prefix=a/".to_string(), "--dst-prefix=b/".to_string()]
        );
    }

    #[test]
    fn a_repository_at_the_workspace_root_needs_no_status_prefix() {
        assert_eq!(workspace_relative_prefix("/workspace", "/workspace"), "");
        assert_eq!(workspace_relative_prefix("/workspace/", "/workspace"), "");
        assert_eq!(
            workspace_relative_prefix("/workspace", "/workspace/moatless"),
            "moatless/"
        );
        // A repo_dir that is not under the workspace at all leaves paths as the
        // repository reported them rather than inventing a prefix.
        assert_eq!(workspace_relative_prefix("/workspace", "/elsewhere"), "");
    }

    #[test]
    fn ahead_and_behind_come_off_the_porcelain_header() {
        assert_eq!(parse_ahead_behind("+2 -3"), (2, 3));
        assert_eq!(parse_ahead_behind("+0 -0"), (0, 0));
        assert_eq!(parse_ahead_behind(""), (0, 0));
    }

    #[test]
    fn a_binary_file_is_a_changed_file_with_no_lines() {
        let mut totals = HashMap::new();
        accumulate_numstat("3\t1\tsrc/main.rs\n-\t-\tlogo.png\n", &mut totals);
        assert_eq!(totals.get("src/main.rs"), Some(&(3, 1)));
        assert_eq!(totals.get("logo.png"), Some(&(0, 0)));
    }

    #[test]
    fn staged_and_unstaged_counts_for_one_file_add_up() {
        let mut totals = HashMap::new();
        accumulate_numstat("1\t0\tf.txt\n", &mut totals);
        accumulate_numstat("2\t5\tf.txt\n", &mut totals);
        assert_eq!(totals.get("f.txt"), Some(&(3, 5)));
    }

    #[test]
    fn one_budget_spans_every_repository_in_a_scope() {
        let mut patch = PatchBuilder::new(10);
        patch.push(&Patch {
            diff: "0123456789abcdef".into(),
            truncated: false,
        });
        assert!(
            patch.is_full(),
            "the first repository used the whole budget"
        );
        patch.push(&Patch {
            diff: "second".into(),
            truncated: false,
        });
        let finished = patch.finish();
        assert!(finished.truncated);
        assert_eq!(finished.diff, format!("0123456789{TRUNCATION_MARKER}"));
    }

    #[test]
    fn patches_within_the_budget_are_joined_whole() {
        let mut patch = PatchBuilder::new(1_000);
        patch.push(&Patch {
            diff: "first\n".into(),
            truncated: false,
        });
        patch.push(&Patch {
            diff: String::new(),
            truncated: false,
        });
        patch.push(&Patch {
            diff: "second\n".into(),
            truncated: false,
        });
        let finished = patch.finish();
        assert!(!finished.truncated);
        assert_eq!(finished.diff, "first\nsecond");
    }

    #[test]
    fn a_cut_never_lands_inside_a_character() {
        let text = "€€€";
        for max in 0..=text.len() {
            let end = char_boundary(text, max);
            assert!(end <= max);
            assert_eq!(end % 3, 0, "{max} cut inside a character");
        }
    }

    /// Against real repositories, because what is under test is what `git`
    /// actually prints — the prefixes it rewrites, the headers it writes for an
    /// untracked file, the porcelain fields it fills in — and none of that is
    /// observable from a mock.
    mod against_git {
        use super::*;
        use std::path::Path;
        use tempfile::TempDir;

        fn git(dir: &Path, args: &[&str]) {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@example.com")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@example.com")
                .output()
                .expect("git runs in the test environment");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        /// A workspace holding one repository in `repo/`, on a `feature`
        /// branch that has committed one change since `main`, with one
        /// uncommitted edit and one untracked file beside it.
        fn workspace() -> TempDir {
            let workspace = TempDir::new().expect("a temp dir");
            let repo = workspace.path().join("repo");
            std::fs::create_dir(&repo).expect("the repo dir");
            git(&repo, &["init", "--initial-branch=main", "-q"]);
            std::fs::write(repo.join("tracked.txt"), "one\ntwo\n").expect("a tracked file");
            git(&repo, &["add", "-A"]);
            git(&repo, &["commit", "-qm", "init"]);

            git(&repo, &["checkout", "-q", "-b", "feature"]);
            std::fs::write(repo.join("committed.txt"), "committed\n").expect("a second file");
            git(&repo, &["add", "-A"]);
            git(&repo, &["commit", "-qm", "on the branch"]);

            std::fs::write(repo.join("tracked.txt"), "one\nTWO\n").expect("an edit");
            std::fs::write(repo.join("untracked.txt"), "fresh\n").expect("an untracked file");
            workspace
        }

        fn root(workspace: &TempDir) -> &str {
            workspace.path().to_str().expect("a utf-8 temp path")
        }

        #[tokio::test]
        async fn every_patch_path_is_workspace_relative() {
            let workspace = workspace();
            let preview = diff_preview(root(&workspace), None, false).await;

            assert!(
                preview
                    .working_tree
                    .diff
                    .contains("diff --git a/repo/tracked.txt b/repo/tracked.txt"),
                "tracked edit is missing its repository prefix: {}",
                preview.working_tree.diff
            );
            assert!(
                preview
                    .working_tree
                    .diff
                    .contains("diff --git a/repo/untracked.txt b/repo/untracked.txt"),
                "untracked file is missing or unprefixed: {}",
                preview.working_tree.diff
            );
            assert!(
                preview.working_tree.diff.contains("new file mode"),
                "an untracked file reads as an addition: {}",
                preview.working_tree.diff
            );
            assert!(
                preview
                    .branch
                    .diff
                    .contains("diff --git a/repo/committed.txt b/repo/committed.txt"),
                "branch scope is missing the committed file: {}",
                preview.branch.diff
            );
            assert!(
                !preview.branch.diff.contains("untracked.txt"),
                "branch scope carries only what is committed: {}",
                preview.branch.diff
            );
        }

        #[tokio::test]
        async fn the_branch_scope_names_the_base_it_resolved() {
            let workspace = workspace();
            let preview = diff_preview(root(&workspace), None, false).await;

            let range = preview.ranges.first().expect("one repository");
            assert_eq!(range.prefix, "repo");
            assert_eq!(range.head_ref.as_deref(), Some("feature"));
            assert_eq!(range.base_ref.as_deref(), Some("main"));
        }

        /// The panel's base picker sends one ref name for the whole workspace,
        /// and a repository that does not have it must not answer with a patch
        /// taken against something else.
        #[tokio::test]
        async fn a_base_that_does_not_resolve_leaves_the_branch_scope_empty() {
            let workspace = workspace();
            let preview = diff_preview(root(&workspace), Some("origin/nope"), false).await;

            assert_eq!(preview.branch.diff, "");
            assert_eq!(
                preview.ranges.first().expect("one repository").base_ref,
                None
            );
        }

        #[tokio::test]
        async fn whitespace_only_edits_disappear_when_they_are_ignored() {
            let workspace = workspace();
            std::fs::write(workspace.path().join("repo/tracked.txt"), "one  \ntwo\n")
                .expect("a whitespace-only edit");

            let header = "diff --git a/repo/tracked.txt b/repo/tracked.txt";
            let kept = diff_preview(root(&workspace), None, false).await;
            assert!(kept.working_tree.diff.contains(header));

            let ignored = diff_preview(root(&workspace), None, true).await;
            assert!(
                !ignored.working_tree.diff.contains(header),
                "a whitespace-only edit survived --ignore-all-space: {}",
                ignored.working_tree.diff
            );
        }

        #[tokio::test]
        async fn status_reads_the_branch_and_its_uncommitted_lines() {
            let workspace = workspace();
            let repo = workspace.path().join("repo");
            let status = status(root(&workspace), repo.to_str().expect("a utf-8 path")).await;

            assert!(status.is_repo);
            assert_eq!(status.ref_name.as_deref(), Some("feature"));
            assert!(!status.is_default_ref);
            assert!(!status.has_primary_remote);
            assert!(!status.has_upstream);
            assert_eq!((status.ahead_count, status.behind_count), (0, 0));
            assert!(status.has_working_tree_changes);
            let tracked = status
                .working_tree_files
                .iter()
                .find(|file| file.path == "repo/tracked.txt")
                .expect("the edited file, at a workspace-relative path");
            assert_eq!((tracked.insertions, tracked.deletions), (1, 1));
        }

        #[tokio::test]
        async fn a_workspace_without_a_repository_has_nothing_to_review() {
            let workspace = TempDir::new().expect("a temp dir");
            std::fs::write(workspace.path().join("loose.txt"), "hi").expect("a loose file");

            let preview = diff_preview(root(&workspace), None, false).await;
            assert_eq!(preview.working_tree.diff, "");
            assert_eq!(preview.branch.diff, "");
            assert!(preview.ranges.is_empty());

            let status = status(root(&workspace), root(&workspace)).await;
            assert!(!status.is_repo);
            let refs = list_refs(root(&workspace)).await;
            assert!(!refs.is_repo);
            assert!(refs.refs.is_empty());
        }

        #[tokio::test]
        async fn refs_carry_which_one_is_checked_out_and_which_is_default() {
            let workspace = workspace();
            let repo = workspace.path().join("repo");
            let listing = list_refs(repo.to_str().expect("a utf-8 path")).await;

            assert!(listing.is_repo);
            assert!(!listing.truncated);
            let mut named: Vec<_> = listing
                .refs
                .iter()
                .map(|reference| {
                    (
                        reference.name.as_str(),
                        reference.current,
                        reference.is_default,
                        reference.is_remote,
                    )
                })
                .collect();
            named.sort_unstable();
            assert_eq!(
                named,
                vec![
                    ("feature", true, false, false),
                    ("main", false, true, false)
                ]
            );
        }
    }
}
