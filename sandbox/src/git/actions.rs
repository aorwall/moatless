//! The stacked-action phase machine: an optional branch, then a commit, a push
//! and a pull request, each run only when the action asks for it and only when
//! it still has work to do, emitting one contract-shaped event per step.
//!
//! Retry-safe by construction: every phase is a skip when its work is already
//! done, so pressing the button again after a failure resumes rather than
//! repeats. Nothing here is rolled back — a hook that rewrote files keeps its
//! work, staged, for the next press.

use std::process::Stdio;

use moatless_api_schemas::github::GithubCreatePrResponse;
use sandbox_proto::git_action_event::Event;
use sandbox_proto::{
    GitActionEvent, GitActionFailed, GitActionFinished, GitActionHookOutput, GitActionPhase,
    GitActionPhaseStarted, GitActionResult, GitActionStarted, GitBranchStep, GitCommitStep,
    GitOutputStream, GitPrStep, GitPushStep, GitStackedAction,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tonic::Status;

use crate::git::generate::{self, GenerationContext};

/// How many bytes of the staged patch to hand the generator, matching how T3
/// caps its own generation context.
const MAX_PATCH_BYTES: usize = 16_000;

/// Everything the action carries, once the request's enum and options are read.
pub struct StackedActionParams {
    pub action: GitStackedAction,
    pub commit_message: Option<String>,
    pub feature_branch: bool,
    pub file_paths: Vec<String>,
}

/// The directories and CLI paths an action reaches, resolved from the sandbox
/// config by the gRPC handler.
pub struct ActionEnv {
    pub repo_dir: String,
    pub claude_cli_path: String,
    pub model: Option<String>,
}

/// Whether the action commits, pushes, or opens a pull request. Each phase runs
/// only when its predicate holds — the phase table in the spec.
fn action_commits(action: GitStackedAction) -> bool {
    matches!(
        action,
        GitStackedAction::Commit | GitStackedAction::CommitPush | GitStackedAction::CommitPushPr
    )
}

fn action_pushes(action: GitStackedAction) -> bool {
    matches!(
        action,
        GitStackedAction::Push | GitStackedAction::CommitPush | GitStackedAction::CommitPushPr
    )
}

fn action_opens_pr(action: GitStackedAction) -> bool {
    matches!(
        action,
        GitStackedAction::CreatePr | GitStackedAction::CommitPushPr
    )
}

/// One phase's decision: run it, or skip it with a reason. Pure — it is the
/// tested core, taking the plan and the runtime facts and nothing else.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Run,
    Skip(&'static str),
}

fn branch_decision(feature_branch: bool) -> Decision {
    if feature_branch {
        Decision::Run
    } else {
        Decision::Skip("skipped_not_requested")
    }
}

fn commit_decision(in_plan: bool, has_staged: bool) -> Decision {
    if !in_plan {
        Decision::Skip("skipped_not_requested")
    } else if !has_staged {
        Decision::Skip("skipped_no_changes")
    } else {
        Decision::Run
    }
}

fn push_decision(in_plan: bool, upstream_at_head: bool) -> Decision {
    if !in_plan {
        Decision::Skip("skipped_not_requested")
    } else if upstream_at_head {
        Decision::Skip("skipped_up_to_date")
    } else {
        Decision::Run
    }
}

fn pr_decision(in_plan: bool) -> Decision {
    if in_plan {
        Decision::Run
    } else {
        Decision::Skip("skipped_not_requested")
    }
}

/// The phases the action announces up front, in order.
fn planned_phases(action: GitStackedAction, feature_branch: bool) -> Vec<i32> {
    let mut phases = Vec::new();
    if feature_branch {
        phases.push(GitActionPhase::Branch as i32);
    }
    if action_commits(action) {
        phases.push(GitActionPhase::Commit as i32);
    }
    if action_pushes(action) {
        phases.push(GitActionPhase::Push as i32);
    }
    if action_opens_pr(action) {
        phases.push(GitActionPhase::Pr as i32);
    }
    phases
}

/// Slugify a commit subject into a branch name a feature branch can carry.
fn branch_name_from_subject(subject: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in subject.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    let slug: String = slug.chars().take(40).collect();
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "t3/change".to_string()
    } else {
        format!("t3/{slug}")
    }
}

/// The first line of a commit message is its subject; the rest is the body.
fn split_message(message: &str) -> (String, Option<String>) {
    let mut lines = message.splitn(2, '\n');
    let subject = lines.next().unwrap_or("").trim().to_string();
    let body = lines
        .next()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(str::to_string);
    (subject, body)
}

async fn emit(tx: &mpsc::Sender<Result<GitActionEvent, Status>>, event: Event) -> bool {
    tx.send(Ok(GitActionEvent { event: Some(event) }))
        .await
        .is_ok()
}

fn phase_started(phase: GitActionPhase, label: &str) -> Event {
    Event::PhaseStarted(GitActionPhaseStarted {
        phase: phase as i32,
        label: label.to_string(),
    })
}

fn failed(phase: GitActionPhase, message: String) -> Event {
    Event::ActionFailed(GitActionFailed {
        phase: Some(phase as i32),
        message,
    })
}

/// Run a git command to completion, capturing its output.
async fn run_git(repo_dir: &str, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git")
        .args(args)
        .current_dir(repo_dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("failed to spawn git: {e}"))
}

/// Stage the named paths, or everything when none were named. Runs before the
/// branch and commit phases so the branch name and the generated message both
/// see the real staged diff; the index carries onto the branch phase's new
/// branch.
async fn stage_changes(repo_dir: &str, file_paths: &[String]) -> Result<(), String> {
    let mut add_args = vec!["add"];
    if file_paths.is_empty() {
        add_args.push("-A");
    } else {
        add_args.push("--");
        add_args.extend(file_paths.iter().map(String::as_str));
    }
    match run_git(repo_dir, &add_args).await {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        Err(e) => Err(e),
    }
}

/// `git diff --cached --quiet` exits non-zero when something is staged.
async fn has_staged_changes(repo_dir: &str) -> bool {
    !run_git(repo_dir, &["diff", "--cached", "--quiet"])
        .await
        .map_or(true, |o| o.status.success())
}

/// Trimmed stdout of a git command, or empty on any failure — for reads whose
/// absence is meaningful rather than an error (no upstream, detached HEAD).
async fn git_line(repo_dir: &str, args: &[&str]) -> String {
    match run_git(repo_dir, args).await {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => String::new(),
    }
}

/// Run `git commit`, relaying every line it and its hooks print as a
/// `hook_output` event. Returns the exit code and the captured stderr so a
/// failure can name what the hook said.
async fn commit_streaming(
    repo_dir: &str,
    args: &[&str],
    tx: &mpsc::Sender<Result<GitActionEvent, Status>>,
) -> Result<(i32, String), String> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(repo_dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to spawn git commit: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "git commit produced no stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "git commit produced no stderr".to_string())?;

    let stdout_task = relay_lines(stdout, GitOutputStream::Stdout, tx.clone());
    let stderr_task = relay_lines(stderr, GitOutputStream::Stderr, tx.clone());
    let (_out, err_text) = tokio::join!(stdout_task, stderr_task);

    let status = child
        .wait()
        .await
        .map_err(|e| format!("git commit did not finish: {e}"))?;
    Ok((status.code().unwrap_or(-1), err_text))
}

/// Read a child stream line by line, emitting each as a `hook_output` event and
/// accumulating the whole into the returned string.
async fn relay_lines<R>(
    reader: R,
    stream: GitOutputStream,
    tx: mpsc::Sender<Result<GitActionEvent, Status>>,
) -> String
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    let mut collected = String::new();
    while let Ok(Some(line)) = lines.next_line().await {
        collected.push_str(&line);
        collected.push('\n');
        let event = Event::HookOutput(GitActionHookOutput {
            hook_name: None,
            stream: stream as i32,
            text: line,
        });
        if tx
            .send(Ok(GitActionEvent { event: Some(event) }))
            .await
            .is_err()
        {
            break;
        }
    }
    collected
}

/// Read up to `MAX_PATCH_BYTES` of the staged patch, on a char boundary.
fn cap_patch(patch: String) -> String {
    if patch.len() <= MAX_PATCH_BYTES {
        return patch;
    }
    let mut end = MAX_PATCH_BYTES;
    while end > 0 && !patch.is_char_boundary(end) {
        end -= 1;
    }
    patch[..end].to_string()
}

async fn generation_context(repo_dir: &str) -> GenerationContext {
    let staged_summary = git_line(repo_dir, &["diff", "--cached", "--stat"]).await;
    let staged_patch = cap_patch(git_line(repo_dir, &["diff", "--cached"]).await);
    let recent_subjects = git_line(repo_dir, &["log", "-10", "--pretty=%s"])
        .await
        .lines()
        .map(str::to_string)
        .collect();
    let pr_template = read_pr_template(repo_dir).await;
    GenerationContext {
        staged_summary,
        staged_patch,
        recent_subjects,
        pr_template,
    }
}

async fn read_pr_template(repo_dir: &str) -> Option<String> {
    for candidate in [
        ".github/pull_request_template.md",
        ".github/PULL_REQUEST_TEMPLATE.md",
        "docs/pull_request_template.md",
    ] {
        let path = format!("{repo_dir}/{candidate}");
        if let Ok(text) = tokio::fs::read_to_string(&path).await {
            if !text.trim().is_empty() {
                return Some(text);
            }
        }
    }
    None
}

/// Drive the whole action, emitting events on `tx`. Returns when the action
/// finishes, fails, or the receiver goes away.
pub async fn run_stacked_action(
    env: ActionEnv,
    params: StackedActionParams,
    tx: mpsc::Sender<Result<GitActionEvent, Status>>,
) {
    let repo_dir = env.repo_dir.as_str();
    let action = params.action;

    let phases = planned_phases(action, params.feature_branch);
    if !emit(&tx, Event::ActionStarted(GitActionStarted { phases })).await {
        return;
    }

    let mut result = GitActionResult {
        branch: Some(GitBranchStep {
            status: "skipped_not_requested".into(),
            name: None,
        }),
        commit: Some(GitCommitStep {
            status: "skipped_not_requested".into(),
            commit_sha: None,
            subject: None,
        }),
        push: Some(GitPushStep {
            status: "skipped_not_requested".into(),
            branch: None,
            upstream_branch: None,
            set_upstream: None,
        }),
        pr: Some(GitPrStep {
            status: "skipped_not_requested".into(),
            url: None,
            number: None,
            base_branch: None,
            head_branch: None,
            title: None,
        }),
    };

    // Stage up front when the action commits, so both the branch name and the
    // generated message read the real staged diff, and a clean worktree is seen
    // as no-changes before any message is generated.
    let mut has_staged = false;
    if action_commits(action) {
        if let Err(msg) = stage_changes(repo_dir, &params.file_paths).await {
            let _ = emit(&tx, failed(GitActionPhase::Commit, msg)).await;
            return;
        }
        has_staged = has_staged_changes(repo_dir).await;
    }

    // Resolve the commit subject/body. Generation runs only when the commit
    // phase will actually commit — staged changes and no message given — so the
    // branch phase can name a branch after it.
    let mut subject_body: Option<(String, Option<String>)> = None;
    if action_commits(action) {
        if let Some(message) = &params.commit_message {
            subject_body = Some(split_message(message));
        } else if commit_decision(true, has_staged) == Decision::Run {
            let ctx = generation_context(repo_dir).await;
            match generate::generate_messages(
                repo_dir,
                &env.claude_cli_path,
                env.model.as_deref(),
                &ctx,
            )
            .await
            {
                Ok(generated) => {
                    subject_body = Some((generated.commit_subject, generated.commit_body));
                }
                Err(reason) => {
                    let _ = emit(&tx, failed(GitActionPhase::Commit, reason)).await;
                    return;
                }
            }
        }
    }

    // ── branch ──────────────────────────────────────────────────────
    if branch_decision(params.feature_branch) == Decision::Run {
        if !emit(
            &tx,
            phase_started(GitActionPhase::Branch, "Creating branch"),
        )
        .await
        {
            return;
        }
        let subject = subject_body.as_ref().map_or("", |(s, _)| s.as_str());
        let name = branch_name_from_subject(subject);
        match run_git(repo_dir, &["checkout", "-b", &name]).await {
            Ok(output) if output.status.success() => {
                result.branch = Some(GitBranchStep {
                    status: "created".into(),
                    name: Some(name),
                });
            }
            Ok(output) => {
                let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let _ = emit(&tx, failed(GitActionPhase::Branch, msg)).await;
                return;
            }
            Err(e) => {
                let _ = emit(&tx, failed(GitActionPhase::Branch, e)).await;
                return;
            }
        }
    }

    // ── commit ──────────────────────────────────────────────────────
    if action_commits(action) {
        if !emit(&tx, phase_started(GitActionPhase::Commit, "Committing")).await {
            return;
        }
        match commit_decision(true, has_staged) {
            Decision::Skip(status) => {
                result.commit = Some(GitCommitStep {
                    status: status.into(),
                    commit_sha: None,
                    subject: None,
                });
            }
            Decision::Run => {
                let (subject, body) = subject_body.clone().unwrap_or_default();
                let mut commit_args = vec!["commit", "-m", subject.as_str()];
                if let Some(body) = &body {
                    commit_args.push("-m");
                    commit_args.push(body.as_str());
                }
                match commit_streaming(repo_dir, &commit_args, &tx).await {
                    Ok((0, _)) => {
                        let sha = git_line(repo_dir, &["rev-parse", "HEAD"]).await;
                        result.commit = Some(GitCommitStep {
                            status: "created".into(),
                            commit_sha: Some(sha).filter(|s| !s.is_empty()),
                            subject: Some(subject),
                        });
                    }
                    Ok((_code, stderr)) => {
                        let msg = if stderr.trim().is_empty() {
                            "git commit failed".to_string()
                        } else {
                            stderr.trim().to_string()
                        };
                        let _ = emit(&tx, failed(GitActionPhase::Commit, msg)).await;
                        return;
                    }
                    Err(e) => {
                        let _ = emit(&tx, failed(GitActionPhase::Commit, e)).await;
                        return;
                    }
                }
            }
        }
    }

    let current_branch = git_line(repo_dir, &["rev-parse", "--abbrev-ref", "HEAD"]).await;

    // ── push ────────────────────────────────────────────────────────
    if action_pushes(action) {
        if !emit(&tx, phase_started(GitActionPhase::Push, "Pushing")).await {
            return;
        }
        let head_sha = git_line(repo_dir, &["rev-parse", "HEAD"]).await;
        let upstream_sha = git_line(repo_dir, &["rev-parse", "@{upstream}"]).await;
        let has_upstream = !upstream_sha.is_empty();
        let upstream_at_head = has_upstream && upstream_sha == head_sha && !head_sha.is_empty();

        match push_decision(true, upstream_at_head) {
            Decision::Skip(status) => {
                result.push = Some(GitPushStep {
                    status: status.into(),
                    branch: Some(current_branch.clone()).filter(|b| !b.is_empty()),
                    upstream_branch: None,
                    set_upstream: None,
                });
            }
            Decision::Run => {
                let set_upstream = !has_upstream;
                let push_args: Vec<&str> = if set_upstream {
                    vec!["push", "-u", "origin", "HEAD"]
                } else {
                    vec!["push"]
                };
                match run_git(repo_dir, &push_args).await {
                    Ok(output) if output.status.success() => {
                        result.push = Some(GitPushStep {
                            status: "pushed".into(),
                            branch: Some(current_branch.clone()).filter(|b| !b.is_empty()),
                            upstream_branch: (!current_branch.is_empty())
                                .then(|| format!("origin/{current_branch}")),
                            set_upstream: Some(set_upstream),
                        });
                    }
                    Ok(output) => {
                        let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
                        let _ = emit(&tx, failed(GitActionPhase::Push, msg)).await;
                        return;
                    }
                    Err(e) => {
                        let _ = emit(&tx, failed(GitActionPhase::Push, e)).await;
                        return;
                    }
                }
            }
        }
    }

    // ── pull request ────────────────────────────────────────────────
    if pr_decision(action_opens_pr(action)) == Decision::Run {
        if !emit(
            &tx,
            phase_started(GitActionPhase::Pr, "Opening pull request"),
        )
        .await
        {
            return;
        }
        let (pr_title, pr_body) = pull_request_message(repo_dir, subject_body.as_ref()).await;
        match open_pull_request(repo_dir, &current_branch, &pr_title, &pr_body).await {
            Ok(pr) => {
                let status = if pr.opened_existing {
                    "opened_existing"
                } else {
                    "created"
                };
                result.pr = Some(GitPrStep {
                    status: status.into(),
                    url: Some(pr.html_url),
                    number: u32::try_from(pr.number).ok(),
                    base_branch: None,
                    head_branch: Some(current_branch.clone()).filter(|b| !b.is_empty()),
                    title: Some(pr.title),
                });
            }
            Err(reason) => {
                let _ = emit(&tx, failed(GitActionPhase::Pr, reason)).await;
                return;
            }
        }
    }

    let _ = emit(
        &tx,
        Event::ActionFinished(GitActionFinished {
            result: Some(result),
        }),
    )
    .await;
}

/// The title and body for the pull request: the generated ones when the action
/// committed, otherwise the head commit's subject and body.
async fn pull_request_message(
    repo_dir: &str,
    subject_body: Option<&(String, Option<String>)>,
) -> (String, String) {
    if let Some((subject, body)) = subject_body {
        return (subject.clone(), body.clone().unwrap_or_default());
    }
    let subject = git_line(repo_dir, &["log", "-1", "--pretty=%s"]).await;
    let body = git_line(repo_dir, &["log", "-1", "--pretty=%b"]).await;
    let title = if subject.is_empty() {
        "Pull request".to_string()
    } else {
        subject
    };
    (title, body)
}

/// Shell out to `moat gh pr create`, which opens the pull request as the Task's
/// run-as user and binds it — the same path the agent uses, so a PR from the
/// toolbar and one from the agent are one binding.
async fn open_pull_request(
    repo_dir: &str,
    head: &str,
    title: &str,
    body: &str,
) -> Result<GithubCreatePrResponse, String> {
    let mut args = vec![
        "gh", "pr", "create", "--title", title, "--body", body, "--json",
    ];
    if !head.is_empty() {
        args.push("--head");
        args.push(head);
    }
    let output = Command::new("moat")
        .args(&args)
        .current_dir(repo_dir)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("failed to run moat gh pr create: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("moat gh pr create failed: {}", stderr.trim()));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("could not read moat gh pr create output: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phase_not_in_the_action_is_skipped_not_requested() {
        assert_eq!(
            commit_decision(false, true),
            Decision::Skip("skipped_not_requested")
        );
        assert_eq!(
            push_decision(false, false),
            Decision::Skip("skipped_not_requested")
        );
        assert_eq!(pr_decision(false), Decision::Skip("skipped_not_requested"));
        assert_eq!(
            branch_decision(false),
            Decision::Skip("skipped_not_requested")
        );
    }

    #[test]
    fn commit_with_nothing_staged_is_skipped_no_changes() {
        assert_eq!(
            commit_decision(true, false),
            Decision::Skip("skipped_no_changes")
        );
        assert_eq!(commit_decision(true, true), Decision::Run);
    }

    #[test]
    fn push_with_upstream_already_at_head_is_skipped_up_to_date() {
        assert_eq!(
            push_decision(true, true),
            Decision::Skip("skipped_up_to_date")
        );
        assert_eq!(push_decision(true, false), Decision::Run);
    }

    #[test]
    fn a_feature_branch_request_runs_the_branch_phase() {
        assert_eq!(branch_decision(true), Decision::Run);
    }

    #[test]
    fn planned_phases_follow_the_action_and_the_branch_flag() {
        assert_eq!(
            planned_phases(GitStackedAction::CommitPushPr, true),
            vec![
                GitActionPhase::Branch as i32,
                GitActionPhase::Commit as i32,
                GitActionPhase::Push as i32,
                GitActionPhase::Pr as i32,
            ]
        );
        assert_eq!(
            planned_phases(GitStackedAction::CreatePr, false),
            vec![GitActionPhase::Pr as i32]
        );
        assert_eq!(
            planned_phases(GitStackedAction::Push, false),
            vec![GitActionPhase::Push as i32]
        );
    }

    #[test]
    fn a_branch_name_is_slugified_from_the_subject() {
        assert_eq!(
            branch_name_from_subject("Fix: the thing!"),
            "t3/fix-the-thing"
        );
        assert_eq!(branch_name_from_subject(""), "t3/change");
        assert_eq!(branch_name_from_subject("   ---   "), "t3/change");
    }

    #[test]
    fn a_message_splits_into_a_subject_and_an_optional_body() {
        assert_eq!(
            split_message("Subject line\n\nBody paragraph"),
            (
                "Subject line".to_string(),
                Some("Body paragraph".to_string())
            )
        );
        assert_eq!(
            split_message("Only a subject"),
            ("Only a subject".to_string(), None)
        );
    }

    /// The phase machine driven against real repositories, so the ordering of
    /// staging, generation, branching and committing is checked where it
    /// actually happens rather than only in the pure decision helpers.
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

        fn read(dir: &Path, args: &[&str]) -> String {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .expect("git runs in the test environment");
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }

        /// A repository on `main` with one commit, plus one uncommitted edit
        /// and one untracked file.
        fn repo() -> TempDir {
            let dir = TempDir::new().expect("a temp dir");
            let path = dir.path();
            git(path, &["init", "--initial-branch=main", "-q"]);
            git(path, &["config", "user.name", "t"]);
            git(path, &["config", "user.email", "t@example.com"]);
            std::fs::write(path.join("tracked.txt"), "one\n").expect("a tracked file");
            git(path, &["add", "-A"]);
            git(path, &["commit", "-qm", "init"]);
            std::fs::write(path.join("tracked.txt"), "one\ntwo\n").expect("an edit");
            std::fs::write(path.join("untracked.txt"), "fresh\n").expect("an untracked file");
            dir
        }

        fn env(dir: &Path) -> ActionEnv {
            ActionEnv {
                repo_dir: dir.to_string_lossy().to_string(),
                // Never reached: every case here supplies a commit message, so
                // generation is not entered.
                claude_cli_path: "/nonexistent/claude".to_string(),
                model: None,
            }
        }

        async fn run(env: ActionEnv, params: StackedActionParams) -> Vec<GitActionEvent> {
            let (tx, mut rx) = mpsc::channel(64);
            run_stacked_action(env, params, tx).await;
            let mut events = Vec::new();
            while let Ok(event) = rx.try_recv() {
                events.push(event.expect("no transport error in-process"));
            }
            events
        }

        fn kinds(events: &[GitActionEvent]) -> Vec<&'static str> {
            events
                .iter()
                .map(|e| match e.event.as_ref().expect("an event body") {
                    Event::ActionStarted(_) => "action_started",
                    Event::PhaseStarted(_) => "phase_started",
                    Event::HookStarted(_) => "hook_started",
                    Event::HookOutput(_) => "hook_output",
                    Event::HookFinished(_) => "hook_finished",
                    Event::ActionFinished(_) => "action_finished",
                    Event::ActionFailed(_) => "action_failed",
                })
                .collect()
        }

        fn finished(events: &[GitActionEvent]) -> GitActionResult {
            match events
                .last()
                .expect("at least one event")
                .event
                .as_ref()
                .expect("an event body")
            {
                Event::ActionFinished(f) => f.result.clone().expect("a result"),
                other => panic!("expected action_finished, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn a_commit_stages_everything_and_lands_on_head() {
            let dir = repo();
            let events = run(
                env(dir.path()),
                StackedActionParams {
                    action: GitStackedAction::Commit,
                    commit_message: Some("feat: a subject\n\nA body.".to_string()),
                    feature_branch: false,
                    file_paths: Vec::new(),
                },
            )
            .await;

            let result = finished(&events);
            let commit = result.commit.expect("a commit step");
            assert_eq!(commit.status, "created");
            assert_eq!(commit.subject.as_deref(), Some("feat: a subject"));
            assert!(commit.commit_sha.is_some(), "the sha is reported");
            assert_eq!(
                read(dir.path(), &["log", "-1", "--pretty=%s"]),
                "feat: a subject"
            );
            // Both the tracked edit and the untracked file were staged.
            assert_eq!(
                read(dir.path(), &["status", "--porcelain"]),
                "",
                "the worktree is clean afterwards"
            );
        }

        #[tokio::test]
        async fn named_paths_are_the_only_ones_committed() {
            let dir = repo();
            let events = run(
                env(dir.path()),
                StackedActionParams {
                    action: GitStackedAction::Commit,
                    commit_message: Some("feat: only the tracked file".to_string()),
                    feature_branch: false,
                    file_paths: vec!["tracked.txt".to_string()],
                },
            )
            .await;

            assert_eq!(
                finished(&events).commit.expect("a commit step").status,
                "created"
            );
            assert_eq!(
                read(dir.path(), &["status", "--porcelain"]),
                "?? untracked.txt",
                "the file nobody named is still untracked"
            );
        }

        #[tokio::test]
        async fn a_clean_worktree_skips_the_commit_without_generating() {
            let dir = TempDir::new().expect("a temp dir");
            let path = dir.path();
            git(path, &["init", "--initial-branch=main", "-q"]);
            git(path, &["config", "user.name", "t"]);
            git(path, &["config", "user.email", "t@example.com"]);
            std::fs::write(path.join("tracked.txt"), "one\n").expect("a tracked file");
            git(path, &["add", "-A"]);
            git(path, &["commit", "-qm", "init"]);

            // No commit message and a clean worktree: generation must not be
            // entered, or the nonexistent CLI path would fail the action.
            let events = run(
                env(path),
                StackedActionParams {
                    action: GitStackedAction::Commit,
                    commit_message: None,
                    feature_branch: false,
                    file_paths: Vec::new(),
                },
            )
            .await;

            assert_eq!(
                finished(&events).commit.expect("a commit step").status,
                "skipped_no_changes"
            );
        }

        #[tokio::test]
        async fn a_feature_branch_is_created_from_the_subject_and_keeps_the_index() {
            let dir = repo();
            let events = run(
                env(dir.path()),
                StackedActionParams {
                    action: GitStackedAction::Commit,
                    commit_message: Some("feat: add a widget".to_string()),
                    feature_branch: true,
                    file_paths: Vec::new(),
                },
            )
            .await;

            let result = finished(&events);
            let branch = result.branch.expect("a branch step");
            assert_eq!(branch.status, "created");
            assert_eq!(branch.name.as_deref(), Some("t3/feat-add-a-widget"));
            assert_eq!(
                read(dir.path(), &["branch", "--show-current"]),
                "t3/feat-add-a-widget"
            );
            // Staging happened before the branch was cut, and the index carried
            // onto it: the commit landed on the new branch, not on main.
            assert_eq!(result.commit.expect("a commit step").status, "created");
            assert_eq!(
                read(dir.path(), &["log", "-1", "--pretty=%s"]),
                "feat: add a widget"
            );
            assert_eq!(
                read(dir.path(), &["log", "-1", "--pretty=%s", "main"]),
                "init",
                "main is untouched"
            );
        }

        #[tokio::test]
        async fn the_first_event_is_action_started_and_the_last_is_terminal() {
            let dir = repo();
            let events = run(
                env(dir.path()),
                StackedActionParams {
                    action: GitStackedAction::Commit,
                    commit_message: Some("feat: ordering".to_string()),
                    feature_branch: false,
                    file_paths: Vec::new(),
                },
            )
            .await;

            let kinds = kinds(&events);
            assert_eq!(kinds.first(), Some(&"action_started"));
            assert_eq!(kinds.last(), Some(&"action_finished"));
            assert!(
                kinds.contains(&"phase_started"),
                "the commit phase announced itself: {kinds:?}"
            );
        }

        #[tokio::test]
        async fn a_push_with_no_remote_fails_naming_the_phase_and_keeps_the_commit() {
            let dir = repo();
            let events = run(
                env(dir.path()),
                StackedActionParams {
                    action: GitStackedAction::CommitPush,
                    commit_message: Some("feat: no remote here".to_string()),
                    feature_branch: false,
                    file_paths: Vec::new(),
                },
            )
            .await;

            assert_eq!(kinds(&events).last(), Some(&"action_failed"));
            match events
                .last()
                .expect("an event")
                .event
                .as_ref()
                .expect("an event body")
            {
                Event::ActionFailed(f) => {
                    assert_eq!(
                        f.phase,
                        Some(GitActionPhase::Push as i32),
                        "the failing phase is named"
                    );
                }
                other => panic!("expected action_failed, got {other:?}"),
            }
            // The commit that landed before the failure stands.
            assert_eq!(
                read(dir.path(), &["log", "-1", "--pretty=%s"]),
                "feat: no remote here"
            );
        }

        /// A hook that prints and a hook that rejects, so the relay is checked
        /// where hooks actually run rather than in a repository that has none.
        fn hook(dir: &Path, name: &str, script: &str) {
            let hooks = dir.join(".git").join("hooks");
            std::fs::create_dir_all(&hooks).expect("the hooks dir");
            let path = hooks.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).expect("the hook file");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .expect("the hook is executable");
            }
        }

        fn hook_lines(events: &[GitActionEvent]) -> Vec<String> {
            events
                .iter()
                .filter_map(|e| match e.event.as_ref().expect("an event body") {
                    Event::HookOutput(o) => Some(o.text.clone()),
                    _ => None,
                })
                .collect()
        }

        #[tokio::test]
        async fn what_a_hook_prints_reaches_the_stream() {
            let dir = repo();
            hook(dir.path(), "pre-commit", "echo 'lint-staged: reformatting'");
            let events = run(
                env(dir.path()),
                StackedActionParams {
                    action: GitStackedAction::Commit,
                    commit_message: Some("feat: with a talkative hook".to_string()),
                    feature_branch: false,
                    file_paths: Vec::new(),
                },
            )
            .await;

            let lines = hook_lines(&events);
            assert!(
                lines
                    .iter()
                    .any(|l| l.contains("lint-staged: reformatting")),
                "the hook's own text is relayed: {lines:?}"
            );
            assert_eq!(
                finished(&events).commit.expect("a commit step").status,
                "created"
            );
        }

        #[tokio::test]
        async fn a_rejecting_hook_fails_the_commit_phase_and_leaves_the_work_staged() {
            let dir = repo();
            hook(
                dir.path(),
                "commit-msg",
                "echo 'subject may not be sentence-case [subject-case]' >&2\nexit 1",
            );
            let events = run(
                env(dir.path()),
                StackedActionParams {
                    action: GitStackedAction::CommitPush,
                    commit_message: Some("Rejected subject".to_string()),
                    feature_branch: false,
                    file_paths: Vec::new(),
                },
            )
            .await;

            match events
                .last()
                .expect("an event")
                .event
                .as_ref()
                .expect("an event body")
            {
                Event::ActionFailed(f) => {
                    assert_eq!(f.phase, Some(GitActionPhase::Commit as i32));
                    assert!(
                        f.message.contains("subject-case"),
                        "the hook's reason is what the person is told: {}",
                        f.message
                    );
                }
                other => panic!("expected action_failed, got {other:?}"),
            }
            // Nothing was committed, and the push never ran.
            assert_eq!(read(dir.path(), &["rev-list", "--count", "HEAD"]), "1");
            assert!(
                !read(dir.path(), &["diff", "--cached", "--name-only"]).is_empty(),
                "the work the hook rejected is still staged, ready for a retry"
            );
        }

        #[tokio::test]
        async fn a_second_run_after_a_failed_push_skips_the_commit_it_already_made() {
            let dir = repo();
            let params = || StackedActionParams {
                action: GitStackedAction::CommitPush,
                commit_message: Some("feat: retry me".to_string()),
                feature_branch: false,
                file_paths: Vec::new(),
            };
            let first = run(env(dir.path()), params()).await;
            assert_eq!(kinds(&first).last(), Some(&"action_failed"));

            let second = run(env(dir.path()), params()).await;
            match second
                .last()
                .expect("an event")
                .event
                .as_ref()
                .expect("an event body")
            {
                Event::ActionFailed(_) => {}
                other => panic!("expected the push to fail again, got {other:?}"),
            }
            assert_eq!(
                read(dir.path(), &["rev-list", "--count", "HEAD"]),
                "2",
                "the retry did not commit a second time"
            );
        }

        /// The generation path against the real agent.
        ///
        /// Every case above supplies a commit message, so none of them enters
        /// `generate::generate_messages` — the one phase that leaves the process
        /// and the one whose output the branch phase then names a branch after.
        /// These run the actual CLI, so they are gated the way the other agent
        /// E2E tests in this repository are:
        ///
        /// ```bash
        /// CLAUDE_SDK_E2E=1 cargo test -p moatless-sandbox git::actions -- --ignored --nocapture
        /// ```
        ///
        /// They need a `CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY` in the
        /// environment, which is what the Sandbox's own secret dispatcher
        /// injects in production.
        mod with_a_real_agent {
            use super::*;

            /// Cheapest model that can hold the output schema.
            const TEST_MODEL: &str = "claude-haiku-4-5-20251001";

            macro_rules! skip_unless_e2e {
                () => {
                    if std::env::var("CLAUDE_SDK_E2E").as_deref() != Ok("1") {
                        eprintln!("Skipping E2E test (set CLAUDE_SDK_E2E=1 to run)");
                        return;
                    }
                };
            }

            fn agent_env(dir: &Path) -> ActionEnv {
                ActionEnv {
                    repo_dir: dir.to_string_lossy().to_string(),
                    claude_cli_path: "claude".to_string(),
                    model: Some(TEST_MODEL.to_string()),
                }
            }

            /// The whole point: no `commit_message`, so the subject and body
            /// come back from the model, and the commit that lands carries them.
            #[tokio::test]
            #[ignore = "calls the real agent"]
            async fn a_generated_message_is_what_the_commit_lands_with() {
                skip_unless_e2e!();
                let dir = repo();
                let events = run(
                    agent_env(dir.path()),
                    StackedActionParams {
                        action: GitStackedAction::Commit,
                        commit_message: None,
                        feature_branch: false,
                        file_paths: Vec::new(),
                    },
                )
                .await;
                let result = finished(&events);
                let commit = result.commit.expect("a commit step");
                assert_eq!(commit.status, "created", "the commit did not land");

                // The model wrote it, so assert the contract rather than the
                // wording: a non-empty subject, and the same text on HEAD.
                let subject = commit.subject.expect("a generated subject");
                assert!(!subject.trim().is_empty(), "the subject was blank");
                assert_eq!(
                    read(dir.path(), &["log", "-1", "--pretty=%s"]),
                    subject,
                    "HEAD's subject is not the generated one"
                );
                assert!(
                    read(dir.path(), &["status", "--porcelain"]).is_empty(),
                    "the worktree was left dirty"
                );
                eprintln!("generated subject: {subject}");
            }

            /// The ordering RC1 fixed, proved from the other side: with real
            /// generation reachable, a clean worktree still reports
            /// `skipped_no_changes` — staging decides, not the model.
            #[tokio::test]
            #[ignore = "calls the real agent"]
            async fn a_clean_worktree_skips_without_asking_the_agent() {
                skip_unless_e2e!();
                let dir = TempDir::new().expect("a temp dir");
                let path = dir.path();
                git(path, &["init", "--initial-branch=main", "-q"]);
                git(path, &["config", "user.name", "t"]);
                git(path, &["config", "user.email", "t@example.com"]);
                std::fs::write(path.join("tracked.txt"), "one\n").expect("a tracked file");
                git(path, &["add", "-A"]);
                git(path, &["commit", "-qm", "init"]);

                let events = run(
                    agent_env(path),
                    StackedActionParams {
                        action: GitStackedAction::Commit,
                        commit_message: None,
                        feature_branch: false,
                        file_paths: Vec::new(),
                    },
                )
                .await;
                let commit = finished(&events).commit.expect("a commit step");
                assert_eq!(commit.status, "skipped_no_changes");
                assert!(commit.subject.is_none(), "a message was generated anyway");
                assert_eq!(
                    read(path, &["rev-list", "--count", "HEAD"]),
                    "1",
                    "the skip still committed"
                );
            }

            /// The branch phase names the branch after the generated subject,
            /// which is only possible because generation runs before it.
            #[tokio::test]
            #[ignore = "calls the real agent"]
            async fn a_feature_branch_is_named_from_the_generated_subject() {
                skip_unless_e2e!();
                let dir = repo();
                let events = run(
                    agent_env(dir.path()),
                    StackedActionParams {
                        action: GitStackedAction::Commit,
                        commit_message: None,
                        feature_branch: true,
                        file_paths: Vec::new(),
                    },
                )
                .await;
                let result = finished(&events);
                let branch = result.branch.expect("a branch step");
                assert_eq!(branch.status, "created");
                let name = branch.name.expect("a branch name");
                let subject = result
                    .commit
                    .expect("a commit step")
                    .subject
                    .expect("a generated subject");
                assert_eq!(
                    name,
                    branch_name_from_subject(&subject),
                    "the branch is not the slug of the generated subject"
                );
                assert_eq!(
                    read(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"]),
                    name
                );
                assert_eq!(
                    read(dir.path(), &["rev-list", "--count", "main"]),
                    "1",
                    "main moved"
                );
                eprintln!("generated branch: {name}");
            }
        }
    }
}
