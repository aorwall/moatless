//! The viewer a browser opening an Artifact URL gets: Moatless's bar above the
//! page, which loads in a frame from the same URL with `?raw=1`.
//!
//! The viewer is ours and runs with the reader's session; the page in its frame
//! is the agent's and keeps its own sandboxing policy, so it can neither reach
//! nor restyle the bar.

use axum::http::HeaderMap;
use serde::Serialize;

use super::service::PageView;
use moatless_types::task::Visibility;

const TEMPLATE: &str = include_str!("shell.html");

/// Whether a page request is a browser opening the URL itself, rather than a
/// frame, a fetch, a download or a client such as the CLI. `raw` forces the page.
pub(super) fn wanted(headers: &HeaderMap, raw: bool, download: bool) -> bool {
    !raw && !download
        && headers
            .get("sec-fetch-dest")
            .is_some_and(|dest| dest.as_bytes() == b"document")
}

/// The viewer's policy: only its own nonce-tagged script and style run, and
/// only this origin is framed and fetched.
pub(super) fn csp(nonce: &str) -> String {
    format!(
        "default-src 'none'; script-src 'nonce-{nonce}'; style-src 'nonce-{nonce}'; \
         img-src 'self' data:; connect-src 'self'; frame-src 'self'; \
         frame-ancestors 'none'; base-uri 'none'; form-action 'none'"
    )
}

/// What the viewer's script renders, for one reader.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShellData<'a> {
    artifact_id: &'a str,
    title: &'a str,
    version: i32,
    latest_version: i32,
    /// The viewer was opened at the unversioned URL.
    latest: bool,
    /// Every published version, oldest first.
    versions: Vec<ShellVersion>,
    visibility: Visibility,
    can_manage: bool,
    /// A fork of the Task can cut at the turn that published this version;
    /// otherwise it cuts at the Task's newest settled turn.
    turn_known: bool,
    /// `None` hides that the Task exists from a reader who cannot read it.
    task: Option<ShellTask>,
    forked_from: Option<ShellSource>,
    home_url: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShellVersion {
    version: i32,
    /// RFC 3339; `None` for a version published before dates were recorded.
    published_at: Option<String>,
}

#[derive(Serialize)]
struct ShellTask {
    url: String,
}

#[derive(Serialize)]
struct ShellSource {
    version: i32,
    /// The source's viewer; `None` when the reader cannot read it.
    url: Option<String>,
}

/// The viewer for `view`, scripted under `nonce`. `frontend_url` is where the
/// web UI's Task pages are, without a trailing slash.
pub(super) fn render(view: &PageView, latest: bool, frontend_url: &str, nonce: &str) -> String {
    let row = &view.row;
    let mut published = row.published_versions.clone();
    published.sort_unstable();
    published.dedup();
    let versions = published
        .into_iter()
        .map(|version| ShellVersion {
            version,
            published_at: view
                .versions
                .iter()
                .find(|recorded| recorded.version == version)
                .map(|recorded| recorded.published_at.to_rfc3339()),
        })
        .collect();
    let data = ShellData {
        artifact_id: &row.artifact_id,
        title: &row.title,
        version: view.version,
        latest_version: row.latest_version,
        latest,
        versions,
        visibility: row.visibility,
        can_manage: view.can_manage,
        turn_known: view
            .versions
            .iter()
            .any(|recorded| recorded.version == view.version && recorded.turn_number.is_some()),
        task: view.task_readable.then(|| ShellTask {
            url: format!("{frontend_url}/tasks/{}", row.task_id),
        }),
        forked_from: row.forked_from_version.map(|version| ShellSource {
            version,
            url: row
                .forked_from_artifact_id
                .as_deref()
                .filter(|_| view.source_readable)
                .map(|source| format!("/api/v1/artifacts/{source}/v{version}")),
        }),
        home_url: frontend_url,
    };
    let data = serde_json::to_string(&data)
        .unwrap_or_else(|error| {
            tracing::error!(%error, "Failed to encode an artifact viewer's data");
            "null".to_string()
        })
        // A `</script>` inside a JSON string still ends the script element.
        .replace('<', "\\u003c");
    fill(
        TEMPLATE,
        &[
            ("__NONCE__", nonce),
            ("__TITLE__", &escape_html(&row.title)),
            ("__DATA__", &data),
        ],
    )
}

/// `template` with each placeholder replaced in one pass over the template,
/// so a value that contains a placeholder's name, such as the agent's title,
/// is never filled in itself.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some((at, name, value)) = values
        .iter()
        .filter_map(|(name, value)| rest.find(name).map(|at| (at, *name, *value)))
        .min_by_key(|(at, _, _)| *at)
    {
        out.push_str(&rest[..at]);
        out.push_str(value);
        rest = &rest[at + name.len()..];
    }
    out.push_str(rest);
    out
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
