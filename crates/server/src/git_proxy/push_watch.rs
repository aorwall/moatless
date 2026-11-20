//! The check that GitHub processed a push it accepted, and the repair when it
//! did not.
//!
//! GitHub can move a branch for a push and never process it: the open pull
//! request keeps the old head, so no checks run and a review gate judges the
//! old commit. Moving the branch back and forth again is processed, so the
//! watch repairs the branch that way, with two updates each conditional on the
//! current head.

use std::sync::Arc;
use std::time::Duration;

use moatless_adapter_github_pr::own_event::{self, Made};
use moatless_loops::bindings::own_events::OwnEvents;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::Deserialize;
use tracing::{error, info, warn};

use crate::github_policy::push::BranchMove;

/// A pack of no objects: what git sends when every commit already exists.
const EMPTY_PACK: &[u8] = b"PACK\0\0\0\x02\0\0\0\0\
    \x02\x9d\x08\x82\x3b\xd8\xa8\xea\xb5\x10\xad\x6a\xc7\x5c\x82\x3c\xfd\x3e\xd3\x1e";

/// How long the watch waits on GitHub.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Timing {
    /// Before each read of a branch and its pull requests. GitHub moves a pull
    /// request's head within seconds of a push it processes.
    pub settle: Duration,
    /// Between the two updates of a repair, so GitHub sees two pushes.
    pub bounce_gap: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            settle: Duration::from_secs(45),
            bounce_gap: Duration::from_secs(5),
        }
    }
}

/// Where the watch reads GitHub, and how long it waits; see
/// [`crate::git_proxy::GitProxy::with_push_watch`].
#[derive(Debug, Clone)]
pub(crate) struct PushWatchConfig {
    pub api_base: String,
    pub timing: Timing,
}

/// One proxied push to a github.com repository, watched until GitHub has
/// processed each branch it moved.
///
/// Detached: a watch lost to a restart only leaves that push unchecked.
pub(super) struct PushWatch {
    pub(super) http: reqwest::Client,
    pub(super) config: PushWatchConfig,
    /// `https://github.com/{owner}/{repo}.git`.
    pub(super) git_url: String,
    pub(super) owner: String,
    pub(super) repo: String,
    pub(super) username: &'static str,
    /// The credential the push itself presented.
    pub(super) token: String,
    pub(super) moves: Vec<BranchMove>,
    /// Where the repair's own pushes are claimed, so their `synchronize` is
    /// not delivered back to the Task.
    pub(super) claim: Option<(Arc<OwnEvents>, String)>,
}

#[derive(Debug, thiserror::Error)]
enum WatchError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("GitHub answered {0}")]
    Status(reqwest::StatusCode),
    #[error("the API base is not a URL")]
    BadBase,
}

/// What a read of a branch and its open pull requests shows for a push.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// The branch is no longer at the pushed commit: a later push moved it, or
    /// this one was rejected. Either way it is not this watch's to check.
    Moved,
    /// Every open pull request on the branch has the pushed head, or there is
    /// none.
    Processed,
    /// The branch is at the pushed commit and these pull requests are not.
    Unprocessed(Vec<u64>),
}

fn verdict(branch_head: Option<&str>, pulls: &[Pull], pushed: &str) -> Verdict {
    if branch_head != Some(pushed) {
        return Verdict::Moved;
    }
    let stale: Vec<u64> = pulls
        .iter()
        .filter(|pull| pull.head.sha != pushed)
        .map(|pull| pull.number)
        .collect();
    if stale.is_empty() {
        Verdict::Processed
    } else {
        Verdict::Unprocessed(stale)
    }
}

#[derive(Debug, Deserialize)]
struct GitRef {
    object: GitObject,
}

#[derive(Debug, Deserialize)]
struct GitObject {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct Pull {
    number: u64,
    head: PullHead,
}

#[derive(Debug, Deserialize)]
struct PullHead {
    sha: String,
}

/// How receive-pack answered one ref update.
#[derive(Debug, PartialEq, Eq)]
enum Update {
    Done,
    /// `ng`, or an unpack failure: the ref is where it was.
    Refused(String),
}

impl PushWatch {
    /// Run the watch in the background.
    pub(super) fn spawn(self) {
        tokio::spawn(async move {
            for branch_move in &self.moves {
                self.watch(branch_move).await;
            }
        });
    }

    async fn watch(&self, pushed: &BranchMove) {
        let mut checks = 0;
        let pulls = loop {
            tokio::time::sleep(self.config.timing.settle).await;
            checks += 1;
            match self.read(pushed).await {
                Ok(Verdict::Unprocessed(pulls)) if checks == 2 => break pulls,
                Ok(Verdict::Unprocessed(_)) => {}
                Ok(Verdict::Moved | Verdict::Processed) => return,
                Err(error) => {
                    warn!(%error, repository = %self.label(), branch = %pushed.branch, "Failed to check that GitHub processed a push");
                    return;
                }
            }
        };
        warn!(
            push_check = "unprocessed",
            repository = %self.label(),
            branch = %pushed.branch,
            sha = %pushed.new,
            pulls = ?pulls,
            "GitHub moved a branch for a push but never processed it; bouncing the branch"
        );
        self.bounce(pushed).await;
    }

    /// Move the branch back to `old` and forward to `new` again.
    ///
    /// # Concurrency
    /// Each update names the head it expects, and receive-pack refuses it when
    /// the branch is elsewhere, so a push that lands mid-repair wins and the
    /// repair stops.
    async fn bounce(&self, pushed: &BranchMove) {
        let repository = self.label();
        let branch = &pushed.branch;
        match self.update(branch, &pushed.new, &pushed.old).await {
            Ok(Update::Done) => {}
            Ok(Update::Refused(reason)) => {
                warn!(push_check = "repair_refused", %repository, %branch, reason, "GitHub refused to move the branch back; the pull request stays on its old head");
                return;
            }
            // A lost answer may follow an applied update, so the branch says
            // which happened.
            Err(error) => match self.branch_head(branch).await {
                Ok(Some(head)) if head == pushed.old => {}
                Ok(Some(head)) if head == pushed.new => {
                    warn!(push_check = "repair_refused", %repository, %branch, %error, "Failed to move the branch back; the pull request stays on its old head");
                    return;
                }
                Ok(_) => {
                    warn!(push_check = "repair_raced", %repository, %branch, %error, "The branch moved during a repair; leaving it to the push that moved it");
                    return;
                }
                Err(read_error) => {
                    error!(push_check = "stranded", %repository, %branch, old = %pushed.old, new = %pushed.new, %error, %read_error, "A repair may have left the branch on its previous commit; push the new one again");
                    return;
                }
            },
        }
        self.claim(&pushed.old).await;

        let mut attempts = 0;
        loop {
            tokio::time::sleep(self.config.timing.bounce_gap).await;
            attempts += 1;
            match self.update(branch, &pushed.old, &pushed.new).await {
                Ok(Update::Done) => break,
                // An earlier attempt whose answer was lost may have applied.
                Ok(Update::Refused(_))
                    if attempts > 1 && self.head_is(branch, &pushed.new).await =>
                {
                    break;
                }
                Ok(Update::Refused(reason)) => {
                    warn!(push_check = "repair_raced", %repository, %branch, reason, "The branch moved during a repair; leaving it to the push that moved it");
                    return;
                }
                Err(_) if attempts == 3 && self.head_is(branch, &pushed.new).await => break,
                Err(error) if attempts == 3 => {
                    error!(push_check = "stranded", %repository, %branch, old = %pushed.old, new = %pushed.new, %error, "A repair left the branch on its previous commit; push the new one again");
                    return;
                }
                Err(error) => {
                    warn!(%repository, %branch, %error, "Failed to move the branch forward again; retrying");
                }
            }
        }

        tokio::time::sleep(self.config.timing.settle).await;
        match self.read(pushed).await {
            Ok(Verdict::Unprocessed(_)) => {
                warn!(push_check = "still_unprocessed", %repository, %branch, "GitHub still has not processed a push after the branch was bounced");
            }
            Ok(Verdict::Moved) => {
                info!(push_check = "moved_after_repair", %repository, %branch, "A later push moved the branch before a repaired push could be checked");
            }
            Ok(Verdict::Processed) => {
                info!(push_check = "repaired", %repository, %branch, "GitHub processed a push after the branch was bounced");
            }
            Err(error) => {
                warn!(%error, %repository, %branch, "Failed to check a repaired push");
            }
        }
    }

    async fn read(&self, pushed: &BranchMove) -> Result<Verdict, WatchError> {
        let head = self.branch_head(&pushed.branch).await?;
        if head.as_deref() != Some(pushed.new.as_str()) {
            return Ok(Verdict::Moved);
        }
        let pulls = self.open_pulls(&pushed.branch).await?;
        Ok(verdict(head.as_deref(), &pulls, &pushed.new))
    }

    fn label(&self) -> String {
        format!("github.com/{}/{}", self.owner, self.repo)
    }

    fn api_url(&self, path: &[&str]) -> Result<reqwest::Url, WatchError> {
        let mut url =
            reqwest::Url::parse(&self.config.api_base).map_err(|_| WatchError::BadBase)?;
        url.path_segments_mut()
            .map_err(|()| WatchError::BadBase)?
            .pop_if_empty()
            .extend(path);
        Ok(url)
    }

    fn api_get(&self, url: reqwest::Url) -> reqwest::RequestBuilder {
        self.http
            .get(url)
            .bearer_auth(&self.token)
            .header(ACCEPT, "application/vnd.github+json")
    }

    /// Whether `branch` reads as `sha`; a failed read answers `false`.
    async fn head_is(&self, branch: &str, sha: &str) -> bool {
        matches!(self.branch_head(branch).await, Ok(Some(head)) if head == sha)
    }

    /// The commit `branch` points at, or `None` when it no longer exists.
    async fn branch_head(&self, branch: &str) -> Result<Option<String>, WatchError> {
        let mut path = vec!["repos", &self.owner, &self.repo, "git", "ref", "heads"];
        path.extend(branch.split('/'));
        let response = self.api_get(self.api_url(&path)?).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(WatchError::Status(response.status()));
        }
        Ok(Some(response.json::<GitRef>().await?.object.sha))
    }

    /// The open pull requests whose head is `branch` in this repository.
    async fn open_pulls(&self, branch: &str) -> Result<Vec<Pull>, WatchError> {
        let url = self.api_url(&["repos", &self.owner, &self.repo, "pulls"])?;
        let head = format!("{}:{branch}", self.owner);
        let response = self
            .api_get(url)
            .query(&[("state", "open"), ("head", &head), ("per_page", "100")])
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(WatchError::Status(response.status()));
        }
        Ok(response.json().await?)
    }

    /// Move `branch` from `from` to `to` over receive-pack, as `git push
    /// --force-with-lease` would.
    async fn update(&self, branch: &str, from: &str, to: &str) -> Result<Update, WatchError> {
        let ref_name = format!("refs/heads/{branch}");
        let response = self
            .http
            .post(format!("{}/git-receive-pack", self.git_url))
            .basic_auth(self.username, Some(&self.token))
            .header(CONTENT_TYPE, "application/x-git-receive-pack-request")
            .header(ACCEPT, "application/x-git-receive-pack-result")
            .body(update_request(&ref_name, from, to))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(WatchError::Status(response.status()));
        }
        Ok(read_report(&response.bytes().await?, &ref_name))
    }

    async fn claim(&self, sha: &str) {
        if let Some((own_events, task_id)) = &self.claim {
            own_events
                .record_best_effort(
                    task_id,
                    own_event::event(&self.owner, &self.repo, &Made::Push(sha.to_string())),
                )
                .await;
        }
    }
}

/// A receive-pack request moving `ref_name` from `from` to `to`, a commit the
/// remote already holds.
fn update_request(ref_name: &str, from: &str, to: &str) -> Vec<u8> {
    let command = format!("{from} {to} {ref_name}\0report-status agent=moatless\n");
    let mut out = format!("{:04x}", command.len() + 4).into_bytes();
    out.extend_from_slice(command.as_bytes());
    out.extend_from_slice(b"0000");
    out.extend_from_slice(EMPTY_PACK);
    out
}

/// `ref_name`'s outcome in a report-status without side-band.
fn read_report(mut bytes: &[u8], ref_name: &str) -> Update {
    while let Some(length) = bytes
        .get(..4)
        .and_then(|h| std::str::from_utf8(h).ok())
        .and_then(|h| usize::from_str_radix(h, 16).ok())
    {
        if length < 4 {
            bytes = &bytes[4..];
            continue;
        }
        let Some(payload) = bytes.get(4..length) else {
            break;
        };
        bytes = &bytes[length..];
        let line = String::from_utf8_lossy(payload);
        let line = line.trim_end_matches('\n');
        if let Some(status) = line.strip_prefix("unpack ") {
            if status != "ok" {
                return Update::Refused(format!("unpack {status}"));
            }
        } else if line.strip_prefix("ok ") == Some(ref_name) {
            return Update::Done;
        } else if let Some(rest) = line.strip_prefix("ng ") {
            let reason = rest.strip_prefix(ref_name).unwrap_or(rest).trim();
            return Update::Refused(reason.to_string());
        }
    }
    Update::Refused("receive-pack reported nothing for the branch".to_string())
}

#[cfg(test)]
mod tests;
