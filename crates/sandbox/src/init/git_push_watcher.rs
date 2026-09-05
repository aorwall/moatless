//! Claims the commit shas this Task pushes to GitHub.
//!
//! A `git push` reaches GitHub directly, so the backend never sees it and
//! nothing names the Task behind the `pull_request.synchronize` webhook that
//! follows. Every other GitHub event a Task produces names its author — a
//! comment through the `Moatless task:` footer, a typed `moat gh` call through
//! the id GitHub returned — and a push had nothing, so it echoed back into the
//! Task that made it.
//!
//! Git's own reflog is the signal. A push writes `update by push` to
//! `.git/logs/refs/remotes/<remote>/<branch>`; a fetch writes `fetch: …` to the
//! same file. Reading the message rather than the ref value is what keeps a
//! fetch of somebody else's commits from being claimed as this Task's work,
//! which would suppress a notification the Task should have received.
//!
//! Timing: the reflog is written as the push completes, and GitHub's webhook
//! still has to travel to the backend and through a durable delivery record
//! before anything reads the claim.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::backend_client::BackendClient;

/// Coalescing window. One `git push` writes one reflog line per ref, so a
/// multi-ref push arrives as a burst.
const DEBOUNCE: Duration = Duration::from_millis(200);

/// The reflog message `git push` writes. A fetch, a pull, a reset and a branch
/// creation all write something else.
const PUSH_REFLOG_MESSAGE: &str = "update by push";

/// The reflog's "no such object" sha, written as the new value when a push
/// deletes the remote branch. Nothing was pushed, so there is nothing to claim.
const ZERO_SHA: &str = "0000000000000000000000000000000000000000";

const GIT_TIMEOUT_SECS: u64 = 5;

/// One checkout being watched.
struct Repo {
    /// Working-tree directory, which `git -C` is run against.
    dir: String,
    /// `<git-dir>/logs/refs/remotes` — the reflogs of every remote-tracking ref.
    remote_logs: PathBuf,
}

/// Spawn the watcher. Returns immediately; it runs until `cancel` fires.
pub fn spawn(
    task_id: String,
    repo_dirs: Vec<String>,
    backend_client: Arc<dyn BackendClient>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(e) = run(task_id, repo_dirs, backend_client, cancel).await {
            warn!(error = %e, "Git push watcher exited with error");
        }
    })
}

async fn run(
    task_id: String,
    repo_dirs: Vec<String>,
    backend_client: Arc<dyn BackendClient>,
    cancel: CancellationToken,
) -> Result<(), String> {
    let mut repos = Vec::new();
    for dir in repo_dirs {
        if let Some(repo) = resolve_repo(&dir).await {
            repos.push(repo);
        } else {
            debug!(dir, "Not a git checkout — not watching it for pushes");
        }
    }
    if repos.is_empty() {
        info!("No git checkouts to watch for pushes");
        return Ok(());
    }

    // Every sha already pushed before this pod started. GitHub delivered those
    // webhooks long ago, so claiming them now would only write dead rows.
    let mut claimed: HashSet<String> = HashSet::new();
    for repo in &repos {
        for push in read_pushes(repo).await {
            claimed.insert(push.sha);
        }
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(32);
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        match res {
            Ok(_) => {
                // Best-effort signal — a full channel already has a rescan
                // pending, and a rescan reads every reflog.
                let _ = tx.try_send(());
            }
            Err(e) => debug!(error = %e, "notify watcher error (transient)"),
        }
    })
    .map_err(|e| format!("failed to create file watcher: {e}"))?;

    for repo in &repos {
        // Recursive: the per-remote subdirectory does not exist until the first
        // push, and a branch name may add more levels under it.
        watcher
            .watch(&repo.remote_logs, RecursiveMode::Recursive)
            .map_err(|e| format!("failed to watch {}: {e}", repo.remote_logs.display()))?;
        info!(dir = %repo.dir, "Watching checkout for pushes");
    }

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!("Git push watcher shutting down");
                return Ok(());
            }
            received = rx.recv() => {
                if received.is_none() {
                    return Ok(());
                }
                let deadline = tokio::time::sleep(DEBOUNCE);
                tokio::pin!(deadline);
                loop {
                    tokio::select! {
                        () = &mut deadline => break,
                        next = rx.recv() => {
                            if next.is_none() {
                                return Ok(());
                            }
                        }
                    }
                }

                claim_new_pushes(&task_id, &repos, &mut claimed, backend_client.as_ref()).await;
            }
        }
    }
}

/// Locate a checkout's remote reflog directory, creating it so the watch handle
/// exists before the first push writes into it.
async fn resolve_repo(dir: &str) -> Option<Repo> {
    let git_dir = git(dir, &["rev-parse", "--absolute-git-dir"]).await?;
    let remote_logs = Path::new(git_dir.trim()).join("logs/refs/remotes");
    if let Err(e) = tokio::fs::create_dir_all(&remote_logs).await {
        warn!(path = %remote_logs.display(), error = %e, "Failed to ensure remote reflog dir");
        return None;
    }
    Some(Repo {
        dir: dir.to_string(),
        remote_logs,
    })
}

/// A sha this checkout pushed, and the remote it went to.
struct Push {
    remote: String,
    sha: String,
}

async fn claim_new_pushes(
    task_id: &str,
    repos: &[Repo],
    claimed: &mut HashSet<String>,
    backend_client: &dyn BackendClient,
) {
    for repo in repos {
        for push in read_pushes(repo).await {
            if claimed.contains(&push.sha) {
                continue;
            }
            let Some(url) = git(
                &repo.dir,
                &["config", "--get", &format!("remote.{}.url", push.remote)],
            )
            .await
            else {
                continue;
            };
            let Some((owner, name)) = github_repo_from_url(url.trim()) else {
                debug!(remote = %push.remote, "Push went to a non-GitHub remote — nothing to claim");
                continue;
            };

            match backend_client
                .record_git_push(task_id, &owner, &name, &push.sha)
                .await
            {
                Ok(()) => {
                    debug!(repo = %format!("{owner}/{name}"), sha = %push.sha, "Claimed pushed sha");
                    claimed.insert(push.sha);
                }
                // Leave it unclaimed so the next push into this repository
                // refiles it. Until it lands, the sha's `synchronize` webhook
                // is delivered back to this Task.
                Err(e) => {
                    warn!(
                        repo = %format!("{owner}/{name}"),
                        sha = %push.sha,
                        error = %e,
                        "Failed to claim pushed sha"
                    );
                }
            }
        }
    }
}

/// Every sha the reflogs under this checkout record as pushed.
async fn read_pushes(repo: &Repo) -> Vec<Push> {
    let mut out = Vec::new();
    let mut stack = vec![repo.remote_logs.clone()];

    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            match entry.file_type().await {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(t) if t.is_file() => {
                    let Some(remote) = remote_name(&repo.remote_logs, &path) else {
                        continue;
                    };
                    let Ok(contents) = tokio::fs::read_to_string(&path).await else {
                        continue;
                    };
                    out.extend(pushed_shas(&contents).into_iter().map(|sha| Push {
                        remote: remote.clone(),
                        sha,
                    }));
                }
                _ => {}
            }
        }
    }
    out
}

/// The remote a reflog file belongs to: the first path component below
/// `logs/refs/remotes`.
fn remote_name(remote_logs: &Path, reflog: &Path) -> Option<String> {
    let relative = reflog.strip_prefix(remote_logs).ok()?;
    let first = relative.components().next()?;
    let name = first.as_os_str().to_str()?;
    // `origin` alone is the reflog of `refs/remotes/origin` itself, which is a
    // symbolic ref to the remote's HEAD and names no branch that was pushed.
    if relative.components().count() < 2 {
        return None;
    }
    Some(name.to_string())
}

/// The new-value shas of a reflog's `update by push` entries.
///
/// Reflog line: `<old-sha> <new-sha> <name> <email> <time> <tz>\t<message>`.
fn pushed_shas(reflog: &str) -> Vec<String> {
    reflog
        .lines()
        .filter_map(|line| {
            let (fields, message) = line.split_once('\t')?;
            if !message.trim().starts_with(PUSH_REFLOG_MESSAGE) {
                return None;
            }
            let sha = fields.split_whitespace().nth(1)?;
            if sha == ZERO_SHA || sha.is_empty() {
                return None;
            }
            Some(sha.to_string())
        })
        .collect()
}

/// `owner` and `repo` of a github.com remote, in any URL form git accepts.
/// `None` for every other host, which this claim says nothing about.
fn github_repo_from_url(url: &str) -> Option<(String, String)> {
    let rest = if let Some(rest) = url.strip_prefix("git@github.com:") {
        rest
    } else {
        let scheme_less = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))
            .or_else(|| url.strip_prefix("ssh://git@"))
            .unwrap_or(url);
        // A credential-bearing URL (`https://x-access-token:TOKEN@github.com/…`)
        // carries the host after the last `@`.
        let host_and_path = scheme_less.rsplit_once('@').map_or(scheme_less, |(_, h)| h);
        host_and_path.strip_prefix("github.com/")?
    };

    let path = rest.trim_end_matches('/').trim_end_matches(".git");
    let (owner, name) = path.split_once('/')?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    Some((owner.to_string(), name.to_string()))
}

/// Run git in `dir`, returning stdout only when it succeeded.
async fn git(dir: &str, args: &[&str]) -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(GIT_TIMEOUT_SECS),
        tokio::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;

    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_push_entry_yields_its_new_sha() {
        let reflog = "0000000000000000000000000000000000000000 171918bb04fd60232e14255d048f529cd9478237 t <t@t> 1788408360 +0000\tupdate by push\n\
                      171918bb04fd60232e14255d048f529cd9478237 b9c0ef1c338ef60e94064536272030d9e7bc498b t <t@t> 1788408361 +0000\tupdate by push\n";
        assert_eq!(
            pushed_shas(reflog),
            vec![
                "171918bb04fd60232e14255d048f529cd9478237".to_string(),
                "b9c0ef1c338ef60e94064536272030d9e7bc498b".to_string(),
            ]
        );
    }

    /// The whole point of reading the message: a fetch advances the same ref
    /// with somebody else's commits, and claiming those would swallow a
    /// notification this Task should receive.
    #[test]
    fn a_fetch_entry_is_not_a_push() {
        let reflog = "0000000000000000000000000000000000000000 b9c0ef1c338ef60e94064536272030d9e7bc498b t <t@t> 1788408360 +0000\tfetch -q origin: storing head\n\
                      b9c0ef1c338ef60e94064536272030d9e7bc498b cafe000000000000000000000000000000000000 t <t@t> 1788408361 +0000\tfetch: fast-forward\n";
        assert!(pushed_shas(reflog).is_empty());
    }

    #[test]
    fn a_branch_deletion_claims_nothing() {
        let reflog = format!(
            "b9c0ef1c338ef60e94064536272030d9e7bc498b {ZERO_SHA} t <t@t> 1788408360 +0000\tupdate by push\n"
        );
        assert!(pushed_shas(&reflog).is_empty());
    }

    #[test]
    fn github_urls_in_every_form_git_accepts() {
        let expected = Some(("soaplabs".to_string(), "soap-knowledge".to_string()));
        for url in [
            "https://github.com/soaplabs/soap-knowledge.git",
            "https://github.com/soaplabs/soap-knowledge",
            "https://github.com/soaplabs/soap-knowledge/",
            "git@github.com:soaplabs/soap-knowledge.git",
            "ssh://git@github.com/soaplabs/soap-knowledge.git",
            "https://x-access-token:ghs_secret@github.com/soaplabs/soap-knowledge.git",
        ] {
            assert_eq!(github_repo_from_url(url), expected, "{url}");
        }
    }

    #[test]
    fn a_non_github_remote_is_not_claimed() {
        assert_eq!(github_repo_from_url("https://gitlab.com/acme/widgets.git"), None);
        assert_eq!(github_repo_from_url("git@bitbucket.org:acme/widgets.git"), None);
        assert_eq!(github_repo_from_url("https://github.com/acme"), None);
    }

    #[test]
    fn the_remote_is_the_first_component_under_the_reflog_root() {
        let root = Path::new("/w/.git/logs/refs/remotes");
        assert_eq!(
            remote_name(root, Path::new("/w/.git/logs/refs/remotes/origin/main")).as_deref(),
            Some("origin")
        );
        assert_eq!(
            remote_name(root, Path::new("/w/.git/logs/refs/remotes/fork/feature/x")).as_deref(),
            Some("fork")
        );
        assert_eq!(
            remote_name(root, Path::new("/w/.git/logs/refs/remotes/origin")),
            None
        );
    }
}
