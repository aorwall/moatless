//! `moat gh pr edit` of a title or body, sent as one REST `PATCH`.
//!
//! Real `gh` edits through a GraphQL mutation that names the pull request by
//! node id alone, so the backend's GitHub proxy cannot tell which repository
//! to mint the token for and presents the primary repository's, which cannot
//! see a pull request anywhere else. The REST path names the repository.

use std::io::Write as _;

use clap::Parser;
use serde::Deserialize;

use super::process::gh_command;
use super::repo::{RepoTarget, parse_pull_request_url, resolve_repo};
use super::typed::read_body;
use crate::client::MoatClient;

/// The `pr edit` flags this path models. Any other flag, or a missing
/// selector, leaves the call to real `gh`.
#[derive(Parser, Debug, PartialEq, Eq)]
#[command(name = "moat gh pr edit")]
pub(super) struct PrEditArgs {
    /// Pull request number or URL.
    selector: String,
    #[arg(long, short = 't')]
    title: Option<String>,
    #[arg(long, short = 'b', conflicts_with = "body_file")]
    body: Option<String>,
    #[arg(long = "body-file", short = 'F')]
    body_file: Option<std::path::PathBuf>,
    /// `owner/repo` (or a GitHub URL).
    #[arg(long = "repo", short = 'R')]
    repo: Option<String>,
}

/// Claim a `pr edit` of a number or URL that changes the title or body and
/// nothing else. A branch selector is left to real `gh`, which resolves it.
pub(super) fn parse(argv: Vec<String>) -> Option<PrEditArgs> {
    PrEditArgs::try_parse_from(argv).ok().filter(|args| {
        let body_file_is_stdin = args.body_file.as_deref() == Some(std::path::Path::new("-"));
        let addressable =
            parse_pull_request_url(&args.selector).is_some() || number(&args.selector).is_some();
        addressable
            && (args.title.is_some() || args.body.is_some() || args.body_file.is_some())
            && !body_file_is_stdin
    })
}

#[derive(Deserialize)]
struct PullRequest {
    html_url: String,
}

pub(super) async fn run(
    client: &MoatClient,
    args: PrEditArgs,
    token: Option<&str>,
    gh_repo: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (target, number) = target(client, &args, gh_repo).await?;
    let mut fields = serde_json::Map::new();
    if let Some(title) = args.title {
        fields.insert("title".into(), title.into());
    }
    if let Some(body) = read_body(args.body, args.body_file.as_deref())? {
        fields.insert("body".into(), body.into());
    }

    let path = format!("repos/{}/pulls/{number}", target.slug());
    let gh_args = ["api", "-X", "PATCH", &path, "--input", "-"].map(String::from);
    let mut child = gh_command(&gh_args, token, None)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run gh: {e} (is the gh CLI installed?)"))?;
    child
        .stdin
        .take()
        .ok_or("failed to open gh's stdin")?
        .write_all(&serde_json::to_vec(&fields)?)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(target.failed(&format!("#{number}"), stderr.trim()));
    }
    let edited: PullRequest = serde_json::from_slice(&output.stdout)?;
    println!("{}", edited.html_url);
    Ok(())
}

/// The repository and number `args` address. A URL names both; a number takes
/// `-R`, else `GH_REPO` as real `gh` would, else the `origin` remote.
async fn target(
    client: &MoatClient,
    args: &PrEditArgs,
    gh_repo: Option<&str>,
) -> Result<(RepoTarget, u64), Box<dyn std::error::Error>> {
    if let Some(url) = parse_pull_request_url(&args.selector) {
        let slug = format!("{}/{}", url.owner, url.repo);
        return Ok((resolve_repo(client, Some(&slug)).await?, url.number));
    }
    let number = number(&args.selector)
        .ok_or_else(|| format!("`{}` is not a pull request number or URL", args.selector))?;
    let flag = args.repo.as_deref().or(gh_repo);
    Ok((resolve_repo(client, flag).await?, number))
}

fn number(selector: &str) -> Option<u64> {
    selector.trim_start_matches('#').parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Option<PrEditArgs> {
        parse(
            std::iter::once("moat gh pr edit")
                .chain(args.iter().copied())
                .map(String::from)
                .collect(),
        )
    }

    #[test]
    fn a_title_or_body_edit_is_claimed() {
        let args = parse_args(&["42", "--body", "new", "-R", "o/r"]).unwrap();
        assert_eq!(args.body.as_deref(), Some("new"));
        assert_eq!(args.repo.as_deref(), Some("o/r"));
        assert!(parse_args(&["42", "-t", "title"]).is_some());
        assert!(parse_args(&["42", "--body-file", "body.md"]).is_some());
    }

    #[test]
    fn anything_else_is_left_to_real_gh() {
        assert!(parse_args(&["42"]).is_none(), "nothing to edit");
        assert!(parse_args(&["--body", "x"]).is_none(), "no selector");
        assert!(parse_args(&["42", "--add-label", "bug"]).is_none());
        assert!(parse_args(&["42", "--body-file", "-"]).is_none(), "stdin");
        assert!(parse_args(&["my-branch", "-t", "x"]).is_none(), "a branch");
    }
}
