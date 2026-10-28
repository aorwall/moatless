//! A rewritten pull request body keeps the PR's `Moatless task:` footer.
//!
//! `pr create` bakes the footer in, and an edit replaces the whole body, so an
//! agent rewriting its description unlinked the PR from its task. [`kept`]
//! copies the footer line the PR already carries; it never invents one.

use std::io::{Read as _, Write as _};

use moatless_types::adapter::FOOTER_MARKER;

use super::process::gh_command_output;
use super::token::github_token_for_gh;

/// `gh pr edit` flags that take no value. Every other flag takes one.
const VALUELESS_FLAGS: &[&str] = &["--remove-milestone", "-h", "--help"];

/// Where the new body comes from.
#[derive(Debug, PartialEq, Eq)]
enum BodySource {
    Inline(String),
    File(String),
}

/// What `keep_task_footer` reads out of a `pr edit` argv.
#[derive(Debug, Default, PartialEq, Eq)]
struct EditArgs {
    /// Inclusive index range of the body flag and its value, after `pr edit`.
    body_span: Option<(usize, usize)>,
    body: Option<BodySource>,
    selector: Option<String>,
    repo: Option<String>,
}

/// What to hand real `gh`: the argv, and the file a rewritten body is written to.
pub(super) struct Passthrough {
    pub(super) args: Vec<String>,
    /// Held until `gh` has run; dropping it deletes the file `args` names.
    _body_file: Option<tempfile::NamedTempFile>,
}

/// Return `args` unchanged, unless it is a `pr edit` with a new body, which
/// [`kept`] gets the footer back on.
///
/// That body goes to `gh` as `--body-file` rather than `--body`: stdin is
/// consumed here, and one argv string is capped at 128 KiB.
pub(super) fn keep_task_footer(
    args: Vec<String>,
    env_token_set: bool,
    gh_repo: Option<&str>,
) -> Result<Passthrough, Box<dyn std::error::Error>> {
    let unchanged = |args| {
        Ok(Passthrough {
            args,
            _body_file: None,
        })
    };
    if args.len() < 2 || args[0] != "pr" || args[1] != "edit" {
        return unchanged(args);
    }
    let edit = parse(&args[2..]);
    let (Some((start, end)), Some(source)) = (edit.body_span, edit.body.as_ref()) else {
        return unchanged(args);
    };
    let new_body = match source {
        BodySource::Inline(b) => b.clone(),
        BodySource::File(path) => read_body_file(path)?,
    };
    let body = kept(new_body, || {
        let token = github_token_for_gh(env_token_set);
        current_body(&edit, token.as_deref(), gh_repo)
    });
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(body.as_bytes())?;
    let mut out = args[..start + 2].to_vec();
    out.push("--body-file".to_string());
    out.push(file.path().to_string_lossy().into_owned());
    out.extend_from_slice(&args[end + 3..]);
    Ok(Passthrough {
        args: out,
        _body_file: Some(file),
    })
}

/// Read the `pr edit` arguments after `pr edit`.
fn parse(rest: &[String]) -> EditArgs {
    let mut edit = EditArgs::default();
    let mut i = 0;
    while i < rest.len() {
        let arg = &rest[i];
        if !arg.starts_with('-') || arg == "-" {
            edit.selector.get_or_insert_with(|| arg.clone());
            i += 1;
            continue;
        }
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        if VALUELESS_FLAGS.contains(&flag) {
            i += 1;
            continue;
        }
        let (value, end) = match inline {
            Some(v) => (Some(v), i),
            None => (rest.get(i + 1).cloned(), i + 1),
        };
        match flag {
            "--body" | "-b" => {
                edit.body = value.map(BodySource::Inline);
                edit.body_span = Some((i, end));
            }
            "--body-file" | "-F" => {
                edit.body = value.map(BodySource::File);
                edit.body_span = Some((i, end));
            }
            "--repo" | "-R" => edit.repo = value,
            _ => {}
        }
        i = end + 1;
    }
    edit
}

fn read_body_file(path: &str) -> Result<String, Box<dyn std::error::Error>> {
    if path == "-" {
        let mut body = String::new();
        std::io::stdin().read_to_string(&mut body)?;
        return Ok(body);
    }
    std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read --body-file {path}: {e}").into())
}

/// `new_body`, with the footer of the body `current` reads appended when it
/// has none of its own. `current` runs only then; a failed read edits without
/// the footer, with a warning.
pub(super) fn kept(
    new_body: String,
    current: impl FnOnce() -> Result<String, Box<dyn std::error::Error>>,
) -> String {
    if footer_line(&new_body).is_some() {
        return new_body;
    }
    match current() {
        Ok(body) => match footer_line(&body) {
            Some(line) => with_footer(&new_body, &line),
            None => new_body,
        },
        Err(e) => {
            eprintln!(
                "moat gh: could not read the current body to keep its `{FOOTER_MARKER}` \
                 footer ({e}); editing without it."
            );
            new_body
        }
    }
}

fn current_body(
    edit: &EditArgs,
    token: Option<&str>,
    gh_repo: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut view = vec!["pr".to_string(), "view".to_string()];
    view.extend(edit.selector.clone());
    if let Some(repo) = &edit.repo {
        view.extend(["--repo".to_string(), repo.clone()]);
    }
    view.extend(["--json", "body", "--jq", ".body"].map(String::from));
    let current = gh_command_output(&view, token, gh_repo)?;
    Ok(String::from_utf8_lossy(&current).into_owned())
}

/// The last `Moatless task:` line of a body.
fn footer_line(body: &str) -> Option<String> {
    body.lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with(FOOTER_MARKER))
        .map(str::to_string)
}

/// Append `line` the way `pr create` lays the footer out.
fn with_footer(body: &str, line: &str) -> String {
    let trimmed = body.trim_end_matches('\n');
    if trimmed.is_empty() {
        format!("{line}\n")
    } else {
        format!("{trimmed}\n\n---\n{line}\n")
    }
}

#[cfg(test)]
mod tests;
