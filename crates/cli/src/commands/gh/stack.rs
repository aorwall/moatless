//! `moat gh stack …` — run the gh-stack extension, then bind what it opened.
//!
//! The payload stays raw so the official extension remains the command grammar
//! owner; only `--task` is moat's, and it is stripped before the call.

use std::collections::HashSet;

use moatless_api_schemas::github::GithubBindPrRequest;

use super::process::{command_status, gh_command, gh_command_output};
use super::repo::{PullRequestRef, RepoTarget, parse_pull_request_url, resolve_repo};
use super::token::github_token_for_gh;
use crate::client::MoatClient;
use crate::config::Config;
use crate::output;

/// `moat gh stack ...` invocation. The payload is intentionally raw so the
/// official gh-stack extension remains the command grammar owner.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct StackInvocation {
    pub(super) args: Vec<String>,
    pub(super) task: Option<String>,
}

pub(super) async fn run_stack(
    client: &MoatClient,
    cfg: &Config,
    invocation: StackInvocation,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let token = github_token_for_gh(cfg.github_token_set);
    let gh_repo = super::repo::origin_gh_repo(client, cfg).await;
    let status = gh_command(&invocation.args, token.as_deref(), gh_repo.as_deref())
        .status()
        .map_err(|e| format!("failed to run gh: {e} (is the gh CLI installed?)"))?;
    if !status.success() {
        return command_status(status.code().unwrap_or(1));
    }

    let Some(subcommand) = invocation.args.get(1).map(String::as_str) else {
        return Ok(());
    };
    if !matches!(subcommand, "link" | "submit") {
        return Ok(());
    }

    let Some(task_id) = non_empty_task_id(
        invocation
            .task
            .or_else(|| cfg.task_id().map(str::to_string)),
    ) else {
        return Ok(());
    };

    let prs = stack_prs_to_bind(
        client,
        &invocation.args,
        token.as_deref(),
        gh_repo.as_deref(),
    )
    .await?;
    if prs.is_empty() {
        return Ok(());
    }

    let mut bound = Vec::new();
    for pr in prs {
        let req = GithubBindPrRequest {
            task_id: task_id.clone(),
        };
        let resp = client
            .api()
            .github_bind_pr(&pr.owner, &pr.repo, pr.number, &req)
            .await?;
        bound.push(resp);
    }

    output::emit(json, &bound, || {
        for pr in &bound {
            println!("Stack PR linked to task: {}", pr.html_url);
            if let Some(bid) = &pr.binding_id {
                println!("Bound to task via: {bid}");
            }
        }
    });

    Ok(())
}

async fn stack_prs_to_bind(
    client: &MoatClient,
    args: &[String],
    token: Option<&str>,
    gh_repo: Option<&str>,
) -> Result<Vec<PullRequestRef>, Box<dyn std::error::Error>> {
    let subcommand = args.get(1).map(String::as_str);
    let RepoTarget { owner, repo, .. } = resolve_repo(client, gh_repo).await?;
    let mut refs = Vec::new();

    match subcommand {
        Some("link") => {
            for operand in stack_link_operands(&args[2..]) {
                if let Some(pr) = parse_pull_request_url(&operand) {
                    refs.push(pr);
                    continue;
                }
                if let Some(pr) = resolve_pr_target(&owner, &repo, &operand, token)? {
                    refs.push(pr);
                }
            }
        }
        Some("submit") => {
            for branch in stack_view_branch_names(token, gh_repo)? {
                if let Some(pr) = resolve_pr_target(&owner, &repo, &branch, token)? {
                    refs.push(pr);
                }
            }
        }
        _ => {}
    }

    let mut seen = HashSet::new();
    refs.retain(|pr| seen.insert(pr.clone()));
    Ok(refs)
}

fn stack_link_operands(args: &[String]) -> Vec<String> {
    let mut operands = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--" => {
                operands.extend(iter.cloned());
                break;
            }
            "--base" | "--remote" => {
                let _ = iter.next();
            }
            "--open" => {}
            value if value.starts_with("--base=") || value.starts_with("--remote=") => {}
            value if value.starts_with('-') => {}
            value => operands.push(value.to_string()),
        }
    }
    operands
}

fn stack_view_branch_names(
    token: Option<&str>,
    gh_repo: Option<&str>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let args = vec![
        "stack".to_string(),
        "view".to_string(),
        "--json".to_string(),
    ];
    let out = gh_command_output(&args, token, gh_repo)?;
    let value: serde_json::Value = serde_json::from_slice(&out)?;
    let branches = value
        .get("branches")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|branch| branch.get("name").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect();
    Ok(branches)
}

fn resolve_pr_target(
    owner: &str,
    repo: &str,
    target: &str,
    token: Option<&str>,
) -> Result<Option<PullRequestRef>, Box<dyn std::error::Error>> {
    let repo_flag = format!("{owner}/{repo}");
    let args = vec![
        "pr".to_string(),
        "view".to_string(),
        target.to_string(),
        "--repo".to_string(),
        repo_flag,
        "--json".to_string(),
        "number,url".to_string(),
    ];
    let out = match gh_command_output(&args, token, None) {
        Ok(out) => out,
        Err(_) => return Ok(None),
    };
    let value: serde_json::Value = serde_json::from_slice(&out)?;
    if let Some(url) = value.get("url").and_then(serde_json::Value::as_str) {
        if let Some(pr) = parse_pull_request_url(url) {
            return Ok(Some(pr));
        }
    }
    let Some(number) = value.get("number").and_then(serde_json::Value::as_u64) else {
        return Ok(None);
    };
    Ok(Some(PullRequestRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
    }))
}

pub(super) fn strip_moat_task_flag(
    args: &[String],
) -> Result<(Vec<String>, Option<String>), String> {
    let mut stripped = Vec::new();
    let mut task = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--task" {
            let Some(value) = iter.next() else {
                return Err("--task requires a value".to_string());
            };
            task = Some(value.to_string());
        } else if let Some(value) = arg.strip_prefix("--task=") {
            if value.is_empty() {
                return Err("--task requires a value".to_string());
            }
            task = Some(value.to_string());
        } else {
            stripped.push(arg.to_string());
        }
    }
    Ok((stripped, task))
}

fn non_empty_task_id(task_id: Option<String>) -> Option<String> {
    task_id.filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests;
