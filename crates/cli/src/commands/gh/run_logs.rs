//! Reading an Actions log through `moat gh`, by the one route that answers.
//!
//! `gh run view --log`/`--log-failed` reads the run's log archive, which is
//! written once the whole run ends and drops steps it cannot map — so the call
//! refuses while any job is still active, and otherwise truncates. The per-job
//! API endpoint answers as soon as that job finishes and returns the whole log,
//! so both paths here go through it.

use clap::Parser;
use serde::Deserialize;
use std::io::Write;

use super::process::gh_command_output;
use super::repo::{current_git_branch, parse_repo_arg};

/// The `run view` flags this path models. Without `--log` or `--log-failed`
/// the call is a status read that real `gh` renders better than we would.
#[derive(Parser, Debug, PartialEq, Eq)]
#[command(name = "moat gh run view")]
pub(super) struct RunLogArgs {
    /// Run id. Defaults to the newest run on the current branch.
    run_id: Option<String>,
    #[arg(long)]
    log: bool,
    #[arg(long = "log-failed")]
    log_failed: bool,
    /// Read one job's log instead of every job in the run.
    #[arg(long)]
    job: Option<u64>,
    /// `owner/repo` (or a GitHub URL).
    #[arg(long = "repo", short = 'R')]
    repo: Option<String>,
}

/// Claim a `run view` invocation, or leave it to real `gh`.
pub(super) fn parse(argv: Vec<String>) -> Option<RunLogArgs> {
    RunLogArgs::try_parse_from(argv)
        .ok()
        .filter(|args| args.log || args.log_failed)
}

/// `args` with `--allow-escape-sequences` added, when it is a `gh api` read of
/// an Actions log that does not already carry it.
///
/// Every Actions log carries ANSI colour, and `gh api` refuses to print a
/// response containing escape sequences without the flag. Older `gh` releases
/// refused silently, which reads as an empty log rather than as a refusal.
pub(super) fn allow_escape_sequences(args: &[String]) -> Option<Vec<String>> {
    if args.iter().any(|arg| arg == "--allow-escape-sequences")
        || !args.iter().any(|arg| reads_actions_log(arg))
    {
        return None;
    }
    let mut out = args.to_vec();
    out.push("--allow-escape-sequences".to_string());
    Some(out)
}

/// Whether `arg` is an endpoint naming a job's or a run's Actions log.
fn reads_actions_log(arg: &str) -> bool {
    let path = arg.split(['?', '#']).next().unwrap_or(arg);
    let path = path.trim_matches('/');
    path.contains("/actions/") && path.rsplit('/').next() == Some("logs")
}

pub(super) fn run(
    args: &RunLogArgs,
    token: Option<&str>,
    gh_repo: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (owner, repo) = resolve_repo(args, gh_repo)?;
    let jobs = match args.job {
        Some(id) => vec![one_job(&owner, &repo, id, token)?],
        None => run_jobs(args, &owner, &repo, token)?,
    };

    let selected: Vec<&Job> = if args.log_failed && args.job.is_none() {
        jobs.iter().filter(|job| job.failed()).collect()
    } else {
        jobs.iter().collect()
    };
    if selected.is_empty() {
        report_no_failure(&jobs);
        return Ok(());
    }

    for job in selected {
        print_job(&owner, &repo, job, token)?;
    }
    Ok(())
}

fn resolve_repo(
    args: &RunLogArgs,
    gh_repo: Option<&str>,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let value = args.repo.as_deref().or(gh_repo).ok_or(
        "could not infer the GitHub repository this run belongs to — pass `-R owner/repo`",
    )?;
    parse_repo_arg(value)
        .ok_or_else(|| format!("invalid repository `{value}` — expected owner/repo").into())
}

#[derive(Deserialize)]
struct JobPage {
    #[serde(default)]
    jobs: Vec<Job>,
}

#[derive(Deserialize)]
struct Job {
    id: u64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    conclusion: Option<String>,
}

impl Job {
    /// Whether this job is one `--log-failed` was asked for. A cancelled job is
    /// left out: it stopped because something else failed, and its log says so.
    fn failed(&self) -> bool {
        matches!(self.conclusion.as_deref(), Some("failure" | "timed_out"))
    }

    fn complete(&self) -> bool {
        self.status == "completed"
    }

    fn outcome(&self) -> &str {
        self.conclusion.as_deref().unwrap_or(&self.status)
    }
}

fn one_job(
    owner: &str,
    repo: &str,
    id: u64,
    token: Option<&str>,
) -> Result<Job, Box<dyn std::error::Error>> {
    let body = api(
        &format!("repos/{owner}/{repo}/actions/jobs/{id}"),
        &[],
        token,
    )
    .map_err(|e| format!("could not read job {id}: {e}"))?;
    serde_json::from_slice(&body).map_err(|e| format!("could not read job {id}: {e}").into())
}

fn run_jobs(
    args: &RunLogArgs,
    owner: &str,
    repo: &str,
    token: Option<&str>,
) -> Result<Vec<Job>, Box<dyn std::error::Error>> {
    let run_id = run_id(args, owner, repo, token)?;
    // `--slurp` is what makes the body parseable: `--paginate` alone prints one
    // JSON object per page, and two pages concatenated parse as nothing.
    let body = api(
        &format!("repos/{owner}/{repo}/actions/runs/{run_id}/jobs"),
        &["--paginate", "--slurp"],
        token,
    )
    .map_err(|e| format!("could not list the jobs in run {run_id}: {e}"))?;
    let pages: Vec<JobPage> = serde_json::from_slice(&body)
        .map_err(|e| format!("could not list the jobs in run {run_id}: {e}"))?;
    let jobs: Vec<Job> = pages.into_iter().flat_map(|page| page.jobs).collect();
    if jobs.is_empty() {
        return Err(format!("run {run_id} has no jobs to read a log from").into());
    }
    Ok(jobs)
}

/// The run to read: the one the caller named, or the newest on this branch.
///
/// Deliberately `gh run list` rather than `gh run view` with no id, which picks
/// a run by prompting and exits non-zero where there is no terminal. The
/// repository is named on the command line because `origin` in a sandbox is a
/// proxy URL `gh` cannot resolve.
fn run_id(
    args: &RunLogArgs,
    owner: &str,
    repo: &str,
    token: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(id) = &args.run_id {
        return Ok(id.clone());
    }
    let branch = current_git_branch()
        .map_err(|e| format!("could not tell which run to read ({e}) — pass the run id"))?;
    let argv = vec![
        "run".to_string(),
        "list".to_string(),
        "--repo".to_string(),
        format!("{owner}/{repo}"),
        "--branch".to_string(),
        branch.clone(),
        "--limit".to_string(),
        "1".to_string(),
        "--json".to_string(),
        "databaseId".to_string(),
        "--jq".to_string(),
        ".[0].databaseId".to_string(),
    ];
    let out = gh_command_output(&argv, token, None)?;
    let id = String::from_utf8_lossy(&out).trim().to_string();
    if id.is_empty() {
        return Err(
            format!("no workflow run on `{branch}` in {owner}/{repo} — pass the run id").into(),
        );
    }
    Ok(id)
}

fn api(
    endpoint: &str,
    extra: &[&str],
    token: Option<&str>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut argv = vec!["api".to_string(), endpoint.to_string()];
    argv.extend(extra.iter().map(ToString::to_string));
    gh_command_output(&argv, token, None)
}

/// Write one job's heading, then its whole log.
///
/// A job that has not finished has no log to read yet, so its heading says so
/// rather than leaving the reader with a heading and nothing under it.
fn print_job(
    owner: &str,
    repo: &str,
    job: &Job,
    token: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !job.complete() {
        println!(
            "==> {} — job id {}, {}, no log until it finishes",
            job.name,
            job.id,
            job.outcome()
        );
        return Ok(());
    }

    println!("==> {} — job id {}, {}", job.name, job.id, job.outcome());
    let log = api(
        &format!("repos/{owner}/{repo}/actions/jobs/{}/logs", job.id),
        &["--allow-escape-sequences"],
        token,
    )
    .map_err(|e| {
        format!(
            "could not read the log of job {} ({}): {e}",
            job.id, job.name
        )
    })?;
    let mut stdout = std::io::stdout();
    stdout.write_all(&log)?;
    if !log.ends_with(b"\n") {
        stdout.write_all(b"\n")?;
    }
    stdout.flush()?;
    Ok(())
}

/// `--log-failed` on a run where nothing failed: say which jobs ran and how
/// they ended, so the answer is never an empty stream.
fn report_no_failure(jobs: &[Job]) {
    println!("no job in this run failed. Jobs:");
    for job in jobs {
        println!("  {}\t{} (job id {})", job.outcome(), job.name, job.id);
    }
}

#[cfg(test)]
mod tests;
