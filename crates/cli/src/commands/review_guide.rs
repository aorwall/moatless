//! `moat review-guide` — hand back the guided review a Task was started to
//! write for a pull request.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use moatless_api_schemas::review_guide::{
    ReviewGuideChange, ReviewGuideFileHunks, ReviewGuideLineRange, ReviewGuideRunResponse,
    ReviewGuideStop, SubmitReviewGuideRequest,
};
use serde::Deserialize;

use crate::client::MoatClient;
use crate::config::Config;
use crate::output;

use super::subprocess;

#[derive(Subcommand)]
pub enum ReviewGuideAction {
    /// Show the pull request and commit range this task's guide describes
    Show,
    /// Submit a guide: a JSON file holding `{"stops": [...]}`.
    ///
    /// Run it from the pull request's checkout. The change is read from git
    /// there, and the guide's line numbers are held to it before it is stored.
    Submit {
        /// The guide JSON file
        path: PathBuf,
    },
}

/// The file an agent writes: the tour, and nothing else.
#[derive(Deserialize)]
struct GuideFile {
    stops: Vec<ReviewGuideStop>,
}

pub async fn run(
    moat: &MoatClient,
    action: ReviewGuideAction,
    cfg: &Config,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let task_id = cfg.require_task_id()?;
    let run = moat.api().review_guide_run(&task_id).await?;
    match action {
        ReviewGuideAction::Show => {
            output::emit(json, &run, || {
                println!(
                    "{}#{}: {}...{}",
                    run.repository, run.number, run.base_sha, run.head_sha
                );
            });
        }
        ReviewGuideAction::Submit { path } => {
            let stops = read_guide(&path)?;
            let change = read_change(&run).await?;
            let req = SubmitReviewGuideRequest { stops, change };
            let resp = moat.api().submit_review_guide(&task_id, &req).await?;
            output::emit(json, &resp, || {
                println!(
                    "Stored the guide: {} stops, {} excerpts.",
                    resp.stops, resp.excerpts
                );
                if resp.dropped_excerpts > 0 {
                    println!(
                        "{} excerpts were dropped: no line of theirs is inside the change.",
                        resp.dropped_excerpts
                    );
                }
                if resp.dropped_notes > 0 {
                    println!(
                        "{} notes were dropped: their line is outside every kept excerpt.",
                        resp.dropped_notes
                    );
                }
                if resp.unplaced_files > 0 {
                    println!(
                        "{} changed files no stop placed were gathered into \"Other files\".",
                        resp.unplaced_files
                    );
                }
            });
        }
    }
    Ok(())
}

fn read_guide(path: &Path) -> Result<Vec<ReviewGuideStop>, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let guide: GuideFile = serde_json::from_str(&text).map_err(|error| {
        format!(
            "{} is not a guide: {error}. Expected {{\"stops\": [{{\"title\", \"summary\", \
             \"attention\", \"excerpts\", \"files\", \"notes\", \"questions\"}}]}}.",
            path.display()
        )
    })?;
    Ok(guide.stops)
}

/// The changed files and lines of `base...head`, read from the checkout in the
/// working directory, fetching the two commits when it lacks them.
async fn read_change(
    run: &ReviewGuideRunResponse,
) -> Result<ReviewGuideChange, Box<dyn std::error::Error>> {
    if !has_commits(run).await? {
        let pull_head = format!("pull/{}/head", run.number);
        // Best effort: a host may refuse a fetch by SHA, which the second
        // fetch covers for the head; the base is on the base branch.
        let _ = git(&["fetch", "--quiet", "origin", &run.base_sha, &run.head_sha]).await?;
        if !has_commits(run).await? {
            let _ = git(&["fetch", "--quiet", "origin", &pull_head]).await?;
        }
        if !has_commits(run).await? {
            return Err(format!(
                "This checkout lacks {} or {}. Run this from a checkout of {}, after \
                 `git fetch origin <base branch> {pull_head}`.",
                run.base_sha, run.head_sha, run.repository
            )
            .into());
        }
    }
    let range = format!("{}...{}", run.base_sha, run.head_sha);
    let names = git(&[
        "-c",
        "core.quotepath=false",
        "diff",
        "--name-only",
        "-z",
        "-M",
        &range,
    ])
    .await?;
    if !names.success() {
        return Err(format!(
            "git diff --name-only {range} failed: {}",
            names.stderr.trim()
        )
        .into());
    }
    let patch = git(&[
        "-c",
        "core.quotepath=false",
        "diff",
        "--unified=0",
        "--no-color",
        "--no-ext-diff",
        "-M",
        &range,
    ])
    .await?;
    if !patch.success() {
        return Err(format!("git diff {range} failed: {}", patch.stderr.trim()).into());
    }
    Ok(ReviewGuideChange {
        files: names
            .stdout
            .split('\0')
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect(),
        hunks: parse_hunks(&patch.stdout),
    })
}

async fn has_commits(run: &ReviewGuideRunResponse) -> Result<bool, Box<dyn std::error::Error>> {
    for sha in [&run.base_sha, &run.head_sha] {
        if !git(&["cat-file", "-e", &format!("{sha}^{{commit}}")])
            .await?
            .success()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn git(args: &[&str]) -> Result<subprocess::CmdOutput, Box<dyn std::error::Error>> {
    Ok(subprocess::run("git", args, None).await?)
}

/// The changed line ranges of each file in a `--unified=0` patch, keyed by the
/// new path, or the old one for a deleted file.
fn parse_hunks(patch: &str) -> BTreeMap<String, ReviewGuideFileHunks> {
    let mut hunks: BTreeMap<String, ReviewGuideFileHunks> = BTreeMap::new();
    let mut old_path: Option<String> = None;
    let mut current: Option<String> = None;
    // Past a file's first `@@`, a removed `-- x` reads `--- x` and an added
    // `++ x` reads `+++ x`: neither is a header there.
    let mut in_hunks = false;
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            old_path = None;
            current = None;
            in_hunks = false;
        } else if let Some(path) = line.strip_prefix("--- ").filter(|_| !in_hunks) {
            old_path = side_path(path, "a/");
        } else if let Some(path) = line.strip_prefix("+++ ").filter(|_| !in_hunks) {
            current = side_path(path, "b/").or_else(|| old_path.clone());
        } else if let (Some(path), Some(header)) = (current.as_ref(), line.strip_prefix("@@ ")) {
            in_hunks = true;
            let Some((old, new)) = parse_hunk_header(header) else {
                continue;
            };
            let file = hunks.entry(path.clone()).or_default();
            file.old.extend(old);
            file.new.extend(new);
        }
    }
    hunks
}

/// A `---`/`+++` path without its `a/`/`b/` prefix; `None` for `/dev/null`.
fn side_path(path: &str, prefix: &str) -> Option<String> {
    let path = path.trim_end_matches('\t');
    let path = path
        .strip_prefix('"')
        .and_then(|quoted| quoted.strip_suffix('"'))
        .unwrap_or(path);
    path.strip_prefix(prefix).map(str::to_string)
}

/// The old and new ranges of `-a,b +c,d @@ ...`; a side with no lines has
/// none.
fn parse_hunk_header(
    header: &str,
) -> Option<(Option<ReviewGuideLineRange>, Option<ReviewGuideLineRange>)> {
    let mut parts = header.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    Some((side_range(side_span(old)?), side_range(side_span(new)?)))
}

/// The `(start, count)` of `a,b`; a bare `a` counts one line.
fn side_span(spec: &str) -> Option<(u32, u32)> {
    match spec.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((spec.parse().ok()?, 1)),
    }
}

fn side_range((start, count): (u32, u32)) -> Option<ReviewGuideLineRange> {
    (count > 0).then(|| ReviewGuideLineRange {
        start,
        end: start + count - 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: u32, end: u32) -> ReviewGuideLineRange {
        ReviewGuideLineRange { start, end }
    }

    #[test]
    fn hunks_are_read_per_side_and_keyed_by_the_surviving_path() {
        let patch = "\
diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -3 +3,2 @@ fn main() {
-old
+new
+more
@@ -10,2 +11,0 @@
-gone
-gone
diff --git a/old.rs b/new.rs
similarity index 90%
rename from old.rs
rename to new.rs
--- a/old.rs
+++ b/new.rs
@@ -0,0 +1 @@
+added
diff --git a/dead.rs b/dead.rs
deleted file mode 100644
--- a/dead.rs
+++ /dev/null
@@ -1,4 +0,0 @@
-x
diff --git a/logo.png b/logo.png
Binary files a/logo.png and b/logo.png differ
";
        let hunks = parse_hunks(patch);
        assert_eq!(
            hunks["src/a.rs"],
            ReviewGuideFileHunks {
                new: vec![range(3, 4)],
                old: vec![range(3, 3), range(10, 11)],
            }
        );
        assert_eq!(hunks["new.rs"].new, vec![range(1, 1)]);
        assert_eq!(hunks["dead.rs"].old, vec![range(1, 4)]);
        assert!(hunks["dead.rs"].new.is_empty());
        assert!(!hunks.contains_key("logo.png"));
    }

    #[test]
    fn hunk_lines_that_look_like_headers_stay_hunk_lines() {
        let patch = "\
diff --git a/schema.sql b/schema.sql
new file mode 100644
--- /dev/null
+++ b/schema.sql
@@ -0,0 +1 @@
+++ counter
@@ -0,0 +4,2 @@
+a
+b
diff --git a/old.sql b/old.sql
--- a/old.sql
+++ b/old.sql
@@ -2 +1,0 @@
--- a comment
@@ -9 +8,0 @@
-x
";
        let hunks = parse_hunks(patch);
        assert_eq!(hunks["schema.sql"].new, vec![range(1, 1), range(4, 5)]);
        assert_eq!(hunks["old.sql"].old, vec![range(2, 2), range(9, 9)]);
    }

    #[test]
    fn a_guide_file_needs_its_stops_key() {
        let dir = std::env::temp_dir().join(format!("guide-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("guide.json");
        std::fs::write(&path, r#"{"stops": [{"title": "One", "summary": "Why."}]}"#).unwrap();
        assert_eq!(read_guide(&path).unwrap()[0].title, "One");
        std::fs::write(&path, r#"[{"title": "One"}]"#).unwrap();
        assert!(read_guide(&path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
