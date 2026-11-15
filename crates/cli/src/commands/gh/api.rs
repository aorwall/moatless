//! `moat gh api` — a passthrough that records what its POST created.
//!
//! A raw `gh api` POST carries no footer and bypasses the typed proxy, so
//! nothing else recognises the comment it creates as the task's own work. The
//! id is read back out of the response and recorded via
//! `/api/v1/github/external-events`.

use std::io::Write;

use moatless_api_schemas::github::{GithubExternalEventRequest, GithubExternalEventSubject};

use super::process::{command_status, gh_command, gh_command_output, passthrough};
use super::token::github_token_for_gh;
use crate::client::MoatClient;
use crate::config::Config;

/// What a raw `gh api` POST creates, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ApiCreation {
    owner: String,
    repo: String,
    /// The pull request or issue the endpoint path names.
    number: u64,
    kind: ApiCreated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApiCreated {
    /// `POST repos/{owner}/{repo}/pulls/{n}/comments` — a new inline review
    /// comment, the shape the reviewer loop's prompt asks agents to use.
    ReviewComment,
    /// `POST repos/{owner}/{repo}/pulls/{n}/reviews` — a review submission.
    Review,
    /// `POST repos/{owner}/{repo}/issues/{n}/comments` — a conversation comment
    /// on a PR or an issue.
    IssueComment,
}

impl ApiCreated {
    /// The fragment GitHub puts before the object's id in its `html_url`.
    fn html_anchor(self) -> &'static str {
        match self {
            Self::ReviewComment => "#discussion_r",
            Self::Review => "#pullrequestreview-",
            Self::IssueComment => "#issuecomment-",
        }
    }

    /// What a call to this endpoint creates, for an error written before it
    /// runs.
    fn describe(self) -> &'static str {
        match self {
            Self::ReviewComment => "review comment",
            Self::Review => "review",
            Self::IssueComment => "comment",
        }
    }

    fn event_subject(self, id: u64) -> GithubExternalEventSubject {
        match self {
            Self::ReviewComment => GithubExternalEventSubject::ReviewComment(id),
            Self::Review => GithubExternalEventSubject::Review(id),
            Self::IssueComment => GithubExternalEventSubject::IssueComment(id),
        }
    }
}

/// Run `gh api …`, and record what a recognised POST created.
///
/// An unrecognised call passes through with inherited stdio at no extra cost; a
/// recognised one captures stdout to read the created id before writing it
/// through unchanged.
pub(super) async fn run_api(
    client: &MoatClient,
    cfg: &Config,
    args: &[String],
    gh_repo: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (args, skip_record) = strip_no_record(args);
    let args = args.as_slice();
    let Some(created) = parse_api_creation(&args[1..]) else {
        return passthrough(cfg, args, gh_repo);
    };
    let Some(task_id) = cfg.task_id().map(str::to_string) else {
        return passthrough(cfg, args, gh_repo);
    };
    if skip_record {
        return passthrough(cfg, args, gh_repo);
    }
    if let Some(flag) = output_replacing_flag(&args[1..]) {
        return Err(refuse_filtered_write(&flag, created.kind).into());
    }

    let token = github_token_for_gh(cfg.github_token_set);
    let output = gh_command(args, token.as_deref(), gh_repo)
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|e| format!("failed to run gh: {e} (is the gh CLI installed?)"))?;

    // Write the response through before anything else: the agent's next step
    // reads it, and a recording failure must not swallow it.
    std::io::stdout().write_all(&output.stdout)?;
    std::io::stdout().flush()?;

    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        match parse_created_id(&text, created.kind) {
            Some(id) => {
                record_event(client, &task_id, &created, created.kind.event_subject(id)).await;

                // The review id recognises none of the comments the same call
                // created; see `review_comment_ids`.
                if created.kind == ApiCreated::Review {
                    for comment_id in review_comment_ids(&created, id, token.as_deref()) {
                        record_event(
                            client,
                            &task_id,
                            &created,
                            GithubExternalEventSubject::ReviewComment(comment_id),
                        )
                        .await;
                    }
                }
            }
            None => eprintln!("moat: gh api response named no id — nothing recorded"),
        }
    }

    command_status(output.status.code().unwrap_or(1))
}

/// Record one created object against the task, and report a failure to stderr.
async fn record_event(
    client: &MoatClient,
    task_id: &str,
    created: &ApiCreation,
    subject: GithubExternalEventSubject,
) {
    let req = GithubExternalEventRequest {
        task_id: task_id.to_string(),
        subject,
        source_name: Some(format!("{}/{}", created.owner, created.repo)),
    };
    if let Err(e) = client.api().github_record_external_event(&req).await {
        // Nothing is suppressed as a result: the event is delivered and the
        // task reads back its own comment, which costs one wasted turn.
        eprintln!("moat: could not record what this call created: {e}");
    }
}

/// The ids of the inline comments a submitted review created.
///
/// Each arrives as its own webhook under its own id, unrecognised by the review
/// id, so the ids are read back from the endpoint that lists them; a failure
/// yields none since the review is already recorded.
fn review_comment_ids(created: &ApiCreation, review_id: u64, token: Option<&str>) -> Vec<u64> {
    // `--slurp` is what makes the body parseable: `--paginate` alone prints one
    // JSON array per page, and two pages concatenated parse as nothing.
    let args = vec![
        "api".to_string(),
        review_comments_endpoint(created, review_id),
        "--paginate".to_string(),
        "--slurp".to_string(),
    ];
    match gh_command_output(&args, token, None) {
        Ok(body) => ids_in_comment_list(&body),
        Err(e) => {
            eprintln!("moat: could not list the review's inline comments: {e}");
            Vec::new()
        }
    }
}

fn review_comments_endpoint(created: &ApiCreation, review_id: u64) -> String {
    format!(
        "repos/{}/{}/pulls/{}/reviews/{review_id}/comments",
        created.owner, created.repo, created.number
    )
}

/// The `id` of every comment in a slurped page list — an array of pages, each
/// itself an array of comments. Anything else yields none.
fn ids_in_comment_list(body: &[u8]) -> Vec<u64> {
    let Ok(serde_json::Value::Array(pages)) = serde_json::from_slice::<serde_json::Value>(body)
    else {
        return Vec::new();
    };
    pages
        .iter()
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .filter_map(|item| item.get("id").and_then(serde_json::Value::as_u64))
        .collect()
}

/// `gh api` flags whose value is the following argument. Skipping that argument
/// is what keeps a value spelled like a flag (`-f body=--jq`) from being read as
/// one.
pub(super) const API_VALUE_FLAGS: &[&str] = &[
    "-f",
    "-F",
    "--field",
    "--raw-field",
    "--input",
    "-H",
    "--header",
    "--method",
    "-X",
    "-q",
    "--jq",
    "-t",
    "--template",
    "-p",
    "--preview",
    "--cache",
    "--hostname",
];

/// The index of every argument that is a flag rather than a value one consumes.
pub(super) fn api_flag_indices(args: &[String]) -> Vec<usize> {
    let mut indices = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg.starts_with('-') {
            indices.push(i);
            if API_VALUE_FLAGS.contains(&arg) {
                i += 1;
            }
        }
        i += 1;
    }
    indices
}

/// The flag that would replace the response body, spelled as the caller wrote
/// it.
///
/// `gh api` prints what the filter yields and nothing else, so the created id is
/// gone before [`parse_created_id`] sees it.
pub(super) fn output_replacing_flag(args: &[String]) -> Option<String> {
    api_flag_indices(args).into_iter().find_map(|i| {
        let arg = args[i].as_str();
        let long = arg.split('=').next().unwrap_or(arg);
        // `-q.id` and `-t{{.id}}`: a short flag carries its value attached, so
        // only the first two characters name it.
        let short = arg.get(..2).unwrap_or(arg);
        (matches!(long, "--jq" | "--template" | "--silent") || matches!(short, "-q" | "-t"))
            .then(|| arg.to_string())
    })
}

/// Take `--no-record` out of the argument list: it is moat's own, and `gh`
/// rejects a flag it does not define.
fn strip_no_record(args: &[String]) -> (Vec<String>, bool) {
    let flags = api_flag_indices(args);
    let mut kept = Vec::with_capacity(args.len());
    let mut found = false;
    for (i, arg) in args.iter().enumerate() {
        if arg == "--no-record" && flags.contains(&i) {
            found = true;
            continue;
        }
        kept.push(arg.clone());
    }
    (kept, found)
}

/// Refuse a creating POST whose response this command cannot read back.
fn refuse_filtered_write(flag: &str, kind: ApiCreated) -> String {
    format!(
        "`{flag}` replaces the response body, and the id of the {} this call \
         creates is read from it — the {} would be posted and recorded against \
         no task. Nothing was posted. Re-run without `{flag}` (the response is \
         written through unchanged either way), or pass `--no-record` to post \
         it unrecorded on purpose.",
        kind.describe(),
        kind.describe()
    )
}

/// Detect a `gh api` argument list that creates a review comment, a review, or
/// an issue comment, and name what it creates.
///
/// Recognised by endpoint shape wherever it sits among the flags; the call must
/// be a POST, since a GET of the same endpoint must never record somebody
/// else's comment.
fn parse_api_creation(args: &[String]) -> Option<ApiCreation> {
    if !api_posts(args) {
        return None;
    }
    args.iter().find_map(|arg| parse_api_endpoint(arg))
}

/// Whether a `gh api` argument list issues a POST.
fn api_posts(args: &[String]) -> bool {
    api_method(args) == "POST"
}

/// The HTTP method a `gh api` argument list sends, upper-cased.
///
/// `gh api` sends GET unless the call carries a body (`-f`/`-F`/`--input`); an
/// explicit `--method`/`-X` overrides either way.
pub(super) fn api_method(args: &[String]) -> String {
    let mut explicit: Option<String> = None;
    let mut has_body = false;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let arg = arg.as_str();
        if arg == "--method" || arg == "-X" {
            explicit = iter.next().map(|v| v.to_ascii_uppercase());
        } else if let Some(value) = arg
            .strip_prefix("--method=")
            .or_else(|| arg.strip_prefix("-X"))
            .filter(|v| !v.is_empty())
        {
            explicit = Some(value.to_ascii_uppercase());
        } else if matches!(arg, "-f" | "-F" | "--field" | "--raw-field" | "--input") {
            has_body = true;
            iter.next();
        } else if (arg.starts_with("-f") || arg.starts_with("-F"))
            || arg.starts_with("--field=")
            || arg.starts_with("--raw-field=")
            || arg.starts_with("--input=")
        {
            has_body = true;
        }
    }

    explicit.unwrap_or_else(|| if has_body { "POST" } else { "GET" }.to_string())
}

/// Decompose a `gh api` endpoint argument into the repository it targets and
/// the kind of event a POST to it creates.
///
/// Accepts bare, rooted, and absolute (`https://api.github.com/...`) spellings,
/// ignoring any query string; a deeper path like `.../comments/{id}/replies`
/// returns `None`.
fn parse_api_endpoint(arg: &str) -> Option<ApiCreation> {
    let path = arg
        .strip_prefix("https://api.github.com/")
        .or_else(|| arg.strip_prefix("http://api.github.com/"))
        .unwrap_or(arg);
    let path = path.split(['?', '#']).next()?;
    let path = path.trim_matches('/');

    let mut parts = path.split('/');
    if parts.next()? != "repos" {
        return None;
    }
    let owner = parts.next()?;
    let repo = parts.next()?;
    let subject = parts.next()?;
    let number = parts.next()?;
    let leaf = parts.next()?;
    if parts.next().is_some() || owner.is_empty() || repo.is_empty() {
        return None;
    }
    let number = number.parse::<u64>().ok()?;

    let kind = match (subject, leaf) {
        ("pulls", "comments") => ApiCreated::ReviewComment,
        ("pulls", "reviews") => ApiCreated::Review,
        ("issues", "comments") => ApiCreated::IssueComment,
        _ => return None,
    };

    Some(ApiCreation {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
        kind,
    })
}

/// The id GitHub gave the object the call created.
///
/// Read from the response's `id` field, or failing that the anchor in its
/// `html_url` — both read the id back out of the response, since one
/// reconstructed from the request would risk silently recording somebody else's
/// event.
fn parse_created_id(text: &str, kind: ApiCreated) -> Option<u64> {
    if let Ok(serde_json::Value::Object(body)) =
        serde_json::from_str::<serde_json::Value>(text.trim())
    {
        if let Some(id) = body.get("id").and_then(serde_json::Value::as_u64) {
            return Some(id);
        }
    }
    last_id_after(text, kind.html_anchor())
}

/// The digits following the last occurrence of `anchor` in `text`.
fn last_id_after(text: &str, anchor: &str) -> Option<u64> {
    let idx = text.rfind(anchor)?;
    let rest = &text[idx + anchor.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

#[cfg(test)]
mod tests;
