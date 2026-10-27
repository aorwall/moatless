//! What `moat gh pr create` puts on a pull request after opening it: the
//! create endpoint takes no labels and no reviewers, so each goes on in a
//! second `gh api` call whose answer is checked for what actually landed.

use super::process::gh_command_output;
use super::token::github_token_for_gh;

/// Put `labels` on the pull request just opened, or fail naming the ones that
/// did not land.
///
/// The response lists what the issue now has, and a label the repository does
/// not define is a 404 there rather than a silent drop.
pub(super) fn apply_labels(
    env_token_set: bool,
    owner: &str,
    repo: &str,
    number: u64,
    labels: &[String],
    html_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut args = vec![
        "api".to_string(),
        format!("repos/{owner}/{repo}/issues/{number}/labels"),
        "--method".to_string(),
        "POST".to_string(),
    ];
    for label in labels {
        args.push("-f".to_string());
        args.push(format!("labels[]={label}"));
    }
    let token = github_token_for_gh(env_token_set);
    match gh_command_output(&args, token.as_deref(), None) {
        Ok(body) => match labels_missing_from(&body, labels) {
            missing if missing.is_empty() => Ok(()),
            missing => Err(label_failure(
                html_url,
                number,
                &missing,
                "the labels GitHub answered with do not include it",
            )
            .into()),
        },
        Err(e) => Err(label_failure(html_url, number, labels, &e.to_string()).into()),
    }
}

/// The requested labels the issue does not carry, by name and case-insensitively
/// as GitHub matches them. A body that does not parse yields every label asked
/// for: an unreadable answer confirms nothing.
fn labels_missing_from(body: &[u8], requested: &[String]) -> Vec<String> {
    let applied: Vec<String> = match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("name").and_then(serde_json::Value::as_str))
            .map(str::to_ascii_lowercase)
            .collect(),
        _ => Vec::new(),
    };
    missing(requested, &applied)
}

fn label_failure(html_url: &str, number: u64, labels: &[String], cause: &str) -> String {
    format!(
        "PR opened and bound at {html_url}, but {} did not land: {cause}. \
         Put it on with `moat gh pr edit {number} --add-label <name>`; a label the \
         repository does not define has to be created first with \
         `moat gh label create <name>`.",
        named(labels)
    )
}

/// Request review from `reviewers` on the pull request just opened, or fail
/// naming the ones GitHub did not record.
///
/// A reviewer is a login, or `org/team` for a team as real `gh` takes it. A
/// login GitHub cannot request (the PR's author, a non-collaborator) fails the
/// whole call with a 422.
pub(super) fn apply_reviewers(
    env_token_set: bool,
    owner: &str,
    repo: &str,
    number: u64,
    reviewers: &[String],
    html_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut args = vec![
        "api".to_string(),
        format!("repos/{owner}/{repo}/pulls/{number}/requested_reviewers"),
        "--method".to_string(),
        "POST".to_string(),
    ];
    for reviewer in reviewers {
        args.push("-f".to_string());
        args.push(match team_slug(reviewer) {
            Some(slug) => format!("team_reviewers[]={slug}"),
            None => format!("reviewers[]={reviewer}"),
        });
    }
    let token = github_token_for_gh(env_token_set);
    match gh_command_output(&args, token.as_deref(), None) {
        Ok(body) => match reviewers_missing_from(&body, reviewers) {
            missing if missing.is_empty() => Ok(()),
            missing => Err(reviewer_failure(
                html_url,
                number,
                &missing,
                "the reviewers GitHub answered with do not include it",
            )
            .into()),
        },
        Err(e) => Err(reviewer_failure(html_url, number, reviewers, &e.to_string()).into()),
    }
}

fn team_slug(reviewer: &str) -> Option<&str> {
    reviewer.split_once('/').map(|(_, slug)| slug)
}

/// The requested reviewers the pull request does not list, matched
/// case-insensitively: a login against `requested_reviewers`, an `org/team`
/// against `requested_teams` by slug. An unreadable body confirms nothing.
fn reviewers_missing_from(body: &[u8], requested: &[String]) -> Vec<String> {
    let pr = serde_json::from_slice::<serde_json::Value>(body).unwrap_or_default();
    let listed = |field: &str, key: &str| -> Vec<String> {
        pr.get(field)
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get(key).and_then(serde_json::Value::as_str))
            .map(str::to_ascii_lowercase)
            .collect()
    };
    let (users, teams) = (
        listed("requested_reviewers", "login"),
        listed("requested_teams", "slug"),
    );
    requested
        .iter()
        .filter(|reviewer| {
            let (key, applied) = match team_slug(reviewer) {
                Some(slug) => (slug, &teams),
                None => (reviewer.as_str(), &users),
            };
            !applied.contains(&key.to_ascii_lowercase())
        })
        .cloned()
        .collect()
}

fn reviewer_failure(html_url: &str, number: u64, reviewers: &[String], cause: &str) -> String {
    format!(
        "PR opened and bound at {html_url}, but review was not requested from {}: \
         {cause}. Request it with `moat gh pr edit {number} --add-reviewer <login>`; \
         GitHub refuses the PR's own author and anyone without access to the repository.",
        named(reviewers)
    )
}

fn missing(requested: &[String], applied: &[String]) -> Vec<String> {
    requested
        .iter()
        .filter(|name| !applied.contains(&name.to_ascii_lowercase()))
        .cloned()
        .collect()
}

fn named(names: &[String]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests;
