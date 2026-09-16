//! `moat gh pr view --comments` — the pull request and its whole conversation.
//!
//! Off a terminal real `gh` prints the comment list alone here: no title, no
//! body, and zero bytes with exit 0 on a pull request nobody has commented on.
//! Every review workflow opens with this command, so that reads as an empty
//! pull request. This path reads the pull request and its three conversation
//! lists from the REST API instead, and fails loudly when one cannot be read.

use clap::Parser;
use serde::Deserialize;
use std::fmt::Write as _;

use super::process::gh_command_output;
use super::repo::parse_pull_request_url;

/// The `pr view` flags this path models.
///
/// Anything else — `--json`, `--jq`, `--template`, `--web` — fails to parse and
/// leaves the call to real `gh`, which already returns data or exits non-zero.
#[derive(Parser, Debug, PartialEq, Eq)]
#[command(name = "moat gh pr view")]
pub(super) struct PrViewArgs {
    /// Pull request number, URL, or branch. Defaults to the current branch's.
    selector: Option<String>,
    /// Render the conversation. Without it the call stays a plain passthrough.
    #[arg(long)]
    comments: bool,
    /// `owner/repo` (or a GitHub URL).
    #[arg(long = "repo", short = 'R')]
    repo: Option<String>,
}

/// The fields the header is rendered from. `url` also names the repository the
/// conversation lists are read from, so a `-R`-less call addresses one repo.
const HEADER_FIELDS: &str =
    "number,title,state,isDraft,author,url,headRefName,baseRefName,createdAt,body";

/// Claim a `pr view` invocation, or leave it to real `gh`.
pub(super) fn parse(argv: Vec<String>) -> Option<PrViewArgs> {
    PrViewArgs::try_parse_from(argv)
        .ok()
        .filter(|args| args.comments)
}

pub(super) fn run(
    args: &PrViewArgs,
    token: Option<&str>,
    gh_repo: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    print!("{}", render(&fetch(args, token, gh_repo)?));
    Ok(())
}

/// A pull request and every comment on it, from the three lists GitHub keeps
/// them in: conversation comments, reviews, and inline review comments.
struct Conversation {
    header: Header,
    comments: Vec<IssueComment>,
    reviews: Vec<Review>,
    review_comments: Vec<ReviewComment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Header {
    number: u64,
    title: String,
    state: String,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    author: Option<GraphqlAuthor>,
    url: String,
    #[serde(default)]
    head_ref_name: String,
    #[serde(default)]
    base_ref_name: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    body: String,
}

#[derive(Deserialize)]
struct GraphqlAuthor {
    #[serde(default)]
    login: String,
}

#[derive(Deserialize)]
struct RestUser {
    #[serde(default)]
    login: String,
}

#[derive(Deserialize)]
struct IssueComment {
    id: u64,
    #[serde(default)]
    user: Option<RestUser>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    created_at: String,
}

#[derive(Deserialize)]
struct Review {
    id: u64,
    #[serde(default)]
    user: Option<RestUser>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    submitted_at: String,
}

#[derive(Deserialize)]
struct ReviewComment {
    id: u64,
    #[serde(default)]
    user: Option<RestUser>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    line: Option<u64>,
    #[serde(default)]
    original_line: Option<u64>,
    #[serde(default)]
    in_reply_to_id: Option<u64>,
}

fn fetch(
    args: &PrViewArgs,
    token: Option<&str>,
    gh_repo: Option<&str>,
) -> Result<Conversation, Box<dyn std::error::Error>> {
    let header = fetch_header(args, token, gh_repo)?;
    let pr = parse_pull_request_url(&header.url).ok_or_else(|| {
        format!(
            "`gh pr view` named no pull request URL to read the conversation from (got `{}`)",
            header.url
        )
    })?;
    let repo = format!("repos/{}/{}", pr.owner, pr.repo);
    let number = header.number;
    Ok(Conversation {
        comments: fetch_list(
            &format!("{repo}/issues/{number}/comments"),
            "conversation comments",
            token,
        )?,
        reviews: fetch_list(&format!("{repo}/pulls/{number}/reviews"), "reviews", token)?,
        review_comments: fetch_list(
            &format!("{repo}/pulls/{number}/comments"),
            "review comments",
            token,
        )?,
        header,
    })
}

fn fetch_header(
    args: &PrViewArgs,
    token: Option<&str>,
    gh_repo: Option<&str>,
) -> Result<Header, Box<dyn std::error::Error>> {
    let mut argv = vec!["pr".to_string(), "view".to_string()];
    if let Some(selector) = &args.selector {
        argv.push(selector.clone());
    }
    if let Some(repo) = &args.repo {
        argv.push("--repo".to_string());
        argv.push(repo.clone());
    }
    argv.push("--json".to_string());
    argv.push(HEADER_FIELDS.to_string());

    let out = gh_command_output(&argv, token, gh_repo)?;
    serde_json::from_slice(&out)
        .map_err(|e| format!("could not read the pull request `gh pr view` returned: {e}").into())
}

/// One paginated REST list, named by `what` so a failure says what is missing.
fn fetch_list<T: serde::de::DeserializeOwned>(
    endpoint: &str,
    what: &str,
    token: Option<&str>,
) -> Result<Vec<T>, Box<dyn std::error::Error>> {
    // `--slurp` is what makes the body parseable: `--paginate` alone prints one
    // JSON array per page, and two pages concatenated parse as nothing.
    let argv = vec![
        "api".to_string(),
        endpoint.to_string(),
        "--paginate".to_string(),
        "--slurp".to_string(),
    ];
    let body = gh_command_output(&argv, token, None)
        .map_err(|e| format!("could not read this pull request's {what}: {e}"))?;
    parse_pages(&body).map_err(|e| {
        format!("could not read this pull request's {what}: unexpected response ({e})").into()
    })
}

/// Flatten `--slurp`'s array of pages, each itself an array of items.
fn parse_pages<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<Vec<T>, serde_json::Error> {
    let pages: Vec<Vec<T>> = serde_json::from_slice(body)?;
    Ok(pages.into_iter().flatten().collect())
}

/// One rendered entry in the conversation, and the instant it sorts on.
enum Entry<'a> {
    Comment(&'a IssueComment),
    Review(&'a Review),
    ReviewComment(&'a ReviewComment),
}

impl Entry<'_> {
    fn at(&self) -> &str {
        match self {
            Self::Comment(c) => &c.created_at,
            Self::Review(r) => &r.submitted_at,
            Self::ReviewComment(c) => &c.created_at,
        }
    }

    /// The line naming who wrote this and which id replies to it.
    ///
    /// The kind is spelled out because the two id spaces take different
    /// commands: `moat gh pr reply` threads under a review comment, and a
    /// conversation comment has no thread to reply in.
    fn heading(&self, number: u64) -> String {
        match self {
            Self::Comment(c) => format!(
                "conversation comment by @{} at {} (comment id: {}) — \
                 reply with `moat gh pr comment {number}`",
                login(c.user.as_ref()),
                stamp(&c.created_at),
                c.id,
            ),
            Self::Review(r) => format!(
                "review by @{} — {} at {} (review id: {})",
                login(r.user.as_ref()),
                if r.state.is_empty() {
                    "SUBMITTED"
                } else {
                    &r.state
                },
                stamp(&r.submitted_at),
                r.id,
            ),
            Self::ReviewComment(c) => {
                let mut heading = format!(
                    "review comment by @{} on {} at {} (comment id: {}) — \
                     reply with `moat gh pr reply {number} {}`",
                    login(c.user.as_ref()),
                    location(c),
                    stamp(&c.created_at),
                    c.id,
                    c.id,
                );
                if let Some(parent) = c.in_reply_to_id {
                    let _ = write!(heading, ", in reply to {parent}");
                }
                heading
            }
        }
    }

    fn body(&self) -> &str {
        match self {
            Self::Comment(c) => &c.body,
            Self::Review(r) => &r.body,
            Self::ReviewComment(c) => &c.body,
        }
    }
}

fn login(user: Option<&RestUser>) -> &str {
    user.map_or("unknown", |u| {
        if u.login.is_empty() {
            "unknown"
        } else {
            &u.login
        }
    })
}

fn stamp(value: &str) -> &str {
    if value.is_empty() {
        "an unknown time"
    } else {
        value
    }
}

fn location(comment: &ReviewComment) -> String {
    let path = if comment.path.is_empty() {
        "an unknown file"
    } else {
        &comment.path
    };
    match comment.line.or(comment.original_line) {
        Some(line) => format!("{path}:{line}"),
        None => path.to_string(),
    }
}

/// The conversation in the order it happened.
///
/// A review carrying no prose is dropped: GitHub wraps every batch of inline
/// comments in one, and those comments are rendered on their own below it.
fn entries(conversation: &Conversation) -> Vec<Entry<'_>> {
    let mut entries: Vec<Entry<'_>> = conversation
        .comments
        .iter()
        .map(Entry::Comment)
        .chain(
            conversation
                .reviews
                .iter()
                .filter(|review| !review.body.trim().is_empty())
                .map(Entry::Review),
        )
        .chain(
            conversation
                .review_comments
                .iter()
                .map(Entry::ReviewComment),
        )
        .collect();
    entries.sort_by(|a, b| a.at().cmp(b.at()));
    entries
}

fn render(conversation: &Conversation) -> String {
    let header = &conversation.header;
    let mut out = String::new();
    let _ = writeln!(out, "number:\t{}", header.number);
    let _ = writeln!(out, "title:\t{}", header.title);
    let _ = writeln!(out, "state:\t{}", header.state);
    let _ = writeln!(out, "draft:\t{}", header.is_draft);
    let _ = writeln!(
        out,
        "author:\t{}",
        header
            .author
            .as_ref()
            .map_or("unknown", |author| if author.login.is_empty() {
                "unknown"
            } else {
                &author.login
            })
    );
    let _ = writeln!(out, "base:\t{}", header.base_ref_name);
    let _ = writeln!(out, "head:\t{}", header.head_ref_name);
    let _ = writeln!(out, "created:\t{}", stamp(&header.created_at));
    let _ = writeln!(out, "url:\t{}", header.url);
    let _ = writeln!(out, "--");
    let _ = writeln!(out, "{}", text_or(&header.body, "(no description)"));

    let entries = entries(conversation);
    if entries.is_empty() {
        let _ = writeln!(out, "--");
        let _ = writeln!(
            out,
            "no comments, reviews, or review comments on this pull request"
        );
        return out;
    }
    for entry in entries {
        let _ = writeln!(out, "--");
        let _ = writeln!(out, "{}", entry.heading(header.number));
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", text_or(entry.body(), "(no body)"));
    }
    out
}

fn text_or<'a>(value: &'a str, empty: &'a str) -> &'a str {
    if value.trim().is_empty() {
        empty
    } else {
        value.trim_end()
    }
}

#[cfg(test)]
mod tests;
