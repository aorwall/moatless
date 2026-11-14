//! A `gh api` call that moves a branch, claimed like a `git push`.
//!
//! The sandbox's push watcher claims a sha from git's reflog, which a commit
//! built through the Git Data or Contents API never writes; unclaimed, the
//! `synchronize` webhook for it is delivered back to the Task that pushed it.

use std::io::Write;

use moatless_api_schemas::github::{GithubExternalEventRequest, GithubExternalEventSubject};

use super::api::{API_VALUE_FLAGS, api_flag_indices, api_method, output_replacing_flag};
use super::process::{command_status, gh_command};
use super::token::github_token_for_gh;
use crate::client::MoatClient;
use crate::config::Config;

/// A call that points a branch at a new commit, and where its sha is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BranchWrite {
    pub(super) owner: String,
    pub(super) repo: String,
    /// JSON pointer to the commit sha in the success response.
    pub(super) sha_at: &'static str,
    /// Whether the request's `sha` field is the commit the branch moves to:
    /// true only for `git/refs`, since `contents`' `sha` names the replaced blob.
    pub(super) sha_in_request: bool,
}

/// Recognise `POST …/git/refs`, `PATCH …/git/refs/{ref}`, `PUT` or `DELETE
/// …/contents/{path}` and `POST …/merges`.
pub(super) fn parse(args: &[String]) -> Option<BranchWrite> {
    let method = api_method(args);
    let flags = api_flag_indices(args);
    args.iter()
        .enumerate()
        .filter(|(i, arg)| !flags.contains(i) && !is_flag_value(args, *i) && !arg.is_empty())
        .find_map(|(_, arg)| parse_endpoint(&method, arg))
}

/// Whether `args[i]` is the value a preceding flag consumes.
fn is_flag_value(args: &[String], i: usize) -> bool {
    i > 0 && API_VALUE_FLAGS.contains(&args[i - 1].as_str())
}

fn parse_endpoint(method: &str, arg: &str) -> Option<BranchWrite> {
    let path = arg
        .strip_prefix("https://api.github.com/")
        .or_else(|| arg.strip_prefix("http://api.github.com/"))
        .unwrap_or(arg);
    let path = path.split(['?', '#']).next()?.trim_matches('/');
    let mut parts = path.splitn(4, '/');
    if parts.next()? != "repos" {
        return None;
    }
    let owner = parts.next().filter(|s| !s.is_empty())?;
    let repo = parts.next().filter(|s| !s.is_empty())?;
    let rest = parts.next()?;

    let sha_at = match (method, rest) {
        ("POST", "git/refs") => "/object/sha",
        ("PATCH", r) if r.strip_prefix("git/refs/").is_some_and(|r| !r.is_empty()) => "/object/sha",
        ("PUT" | "DELETE", r) if r.strip_prefix("contents/").is_some_and(|r| !r.is_empty()) => {
            "/commit/sha"
        }
        ("POST", "merges") => "/sha",
        _ => return None,
    };
    Some(BranchWrite {
        owner: owner.to_string(),
        repo: repo.to_string(),
        sha_at,
        sha_in_request: rest.starts_with("git/refs"),
    })
}

/// The commit sha the branch now points at: from the response, or from a
/// `-f sha=…` field when a filter replaced the response.
fn pushed_sha(
    response: &[u8],
    filtered: bool,
    args: &[String],
    write: &BranchWrite,
) -> Option<String> {
    if !filtered {
        return response_json(response)?
            .pointer(write.sha_at)?
            .as_str()
            .map(str::to_string);
    }
    if !write.sha_in_request {
        return None;
    }
    args.windows(2).find_map(|pair| {
        matches!(pair[0].as_str(), "-f" | "-F" | "--field" | "--raw-field")
            .then(|| pair[1].strip_prefix("sha="))
            .flatten()
            .filter(|sha| !sha.is_empty())
            .map(str::to_string)
    })
}

/// The response body as JSON, past the status line and headers `--include`
/// prints ahead of it.
fn response_json(response: &[u8]) -> Option<serde_json::Value> {
    serde_json::from_slice(response).ok().or_else(|| {
        let text = std::str::from_utf8(response).ok()?;
        let (_, body) = text
            .split_once("\r\n\r\n")
            .or_else(|| text.split_once("\n\n"))?;
        serde_json::from_str(body).ok()
    })
}

/// Run the call, write its output through, and claim the sha it pushed.
pub(super) async fn run(
    client: &MoatClient,
    cfg: &Config,
    args: &[String],
    gh_repo: Option<&str>,
    task_id: &str,
    write: &BranchWrite,
) -> Result<(), Box<dyn std::error::Error>> {
    let token = github_token_for_gh(cfg.github_token_set);
    let output = gh_command(args, token.as_deref(), gh_repo)
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|e| format!("failed to run gh: {e} (is the gh CLI installed?)"))?;
    std::io::stdout().write_all(&output.stdout)?;
    std::io::stdout().flush()?;

    if output.status.success() {
        let filtered = output_replacing_flag(&args[1..]).is_some();
        match pushed_sha(&output.stdout, filtered, &args[1..], write) {
            Some(sha) => {
                let req = GithubExternalEventRequest {
                    task_id: task_id.to_string(),
                    subject: GithubExternalEventSubject::Push(sha),
                    source_name: Some(format!("{}/{}", write.owner, write.repo)),
                };
                if let Err(e) = client.api().github_record_external_event(&req).await {
                    eprintln!("moat: could not claim the commit this call pushed: {e}");
                }
            }
            None => eprintln!(
                "moat: could not read the pushed commit's sha, so this Task will hear its own push"
            ),
        }
    }
    command_status(output.status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(line: &str) -> Option<BranchWrite> {
        let args: Vec<String> = line.split_whitespace().map(ToString::to_string).collect();
        parse(&args[1..])
    }

    #[test]
    fn every_api_call_that_moves_a_branch_is_recognised() {
        let at = |line: &str| parsed(line).map(|w| (w.owner, w.repo, w.sha_at));
        let refs = Some(("o".to_string(), "r".to_string(), "/object/sha"));
        assert_eq!(
            at("api -X POST repos/o/r/git/refs -f ref=refs/heads/b -f sha=abc"),
            refs
        );
        assert_eq!(
            at("api -f ref=refs/heads/b -f sha=abc /repos/o/r/git/refs"),
            refs
        );
        assert_eq!(
            at("api -X PATCH repos/o/r/git/refs/heads/feat/x -f sha=abc -F force=true"),
            refs
        );
        assert_eq!(
            at("api -X PUT repos/o/r/contents/docs/a.md -f message=m -f content=Yg=="),
            Some(("o".into(), "r".into(), "/commit/sha"))
        );
        assert_eq!(
            at("api -X POST https://api.github.com/repos/o/r/merges -f base=b -f head=main"),
            Some(("o".into(), "r".into(), "/sha"))
        );
    }

    #[test]
    fn reads_and_writes_that_move_no_branch_claim_nothing() {
        assert!(parsed("api repos/o/r/git/refs/heads/b").is_none());
        assert!(parsed("api -X DELETE repos/o/r/git/refs/heads/b").is_none());
        assert!(parsed("api -X POST repos/o/r/git/commits --input c.json").is_none());
        assert!(parsed("api -X POST repos/o/r/git/blobs -f content=x").is_none());
        assert!(parsed("api repos/o/r/contents/a.md").is_none());
        // A field value spelled like an endpoint is not one.
        assert!(parsed("api -X POST repos/o/r/issues -f body=repos/o/r/git/refs").is_none());
    }

    #[test]
    fn the_sha_is_read_from_the_response_or_a_filtered_calls_sha_field() {
        let write = parsed("api -X PATCH repos/o/r/git/refs/heads/b -f sha=req").unwrap();
        let args: Vec<String> = ["-X", "PATCH", "repos/o/r/git/refs/heads/b", "-f", "sha=req"]
            .map(String::from)
            .to_vec();
        let body = br#"{"ref":"refs/heads/b","object":{"sha":"resp","type":"commit"}}"#;
        assert_eq!(
            pushed_sha(body, false, &args, &write).as_deref(),
            Some("resp")
        );
        assert_eq!(
            pushed_sha(b".object.sha", true, &args, &write).as_deref(),
            Some("req")
        );
        assert_eq!(pushed_sha(b"not json", false, &args, &write), None);

        let included = b"HTTP/2.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"object\":{\"sha\":\"resp\"}}";
        assert_eq!(
            pushed_sha(included, false, &args, &write).as_deref(),
            Some("resp")
        );
    }

    /// A filtered `contents` write's `sha` field names the blob it replaces,
    /// so nothing is claimed rather than a sha no push carries.
    #[test]
    fn a_filtered_contents_write_claims_nothing_from_its_blob_sha() {
        let line = "api -X PUT repos/o/r/contents/x --jq .content.path -f sha=blob";
        let write = parsed(line).unwrap();
        let args: Vec<String> = line.split_whitespace().skip(1).map(String::from).collect();
        assert_eq!(pushed_sha(b"x", true, &args, &write), None);
    }
}
