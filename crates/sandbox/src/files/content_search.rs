//! Searching the workspace's file contents, by running ripgrep over it.
//!
//! ripgrep rather than a walk here: the sandbox image ships it, and it already
//! applies each repository's ignore rules the way an agent's own search does.

use std::process::Stdio;
use std::time::Duration;

use sandbox_proto::UNBROWSABLE_DIRS;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tracing::warn;

const DEFAULT_MAX_RESULTS: usize = 100;
const HARD_MAX_RESULTS: usize = 500;

/// How long a search runs before it answers with what it has, truncated.
const SEARCH_DEADLINE: Duration = Duration::from_secs(15);

/// The most matching lines one file contributes, so one generated file cannot
/// fill the page.
const MAX_MATCHES_PER_FILE: &str = "20";

/// Larger files are skipped: they are generated or data, not code to read.
const MAX_FILE_SIZE: &str = "1M";

/// The most characters of a matching line carried; ranges past the cut drop.
const MAX_LINE_CHARS: usize = 500;

/// One content search, as the client asked for it.
#[derive(Debug)]
pub struct ContentQuery<'a> {
    /// Searched as written; whitespace is significant.
    pub query: &'a str,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub use_regex: bool,
    /// 0 asks for the default.
    pub max_results: i32,
}

/// One matching line.
#[derive(Debug, PartialEq, Eq)]
pub struct ContentMatch {
    /// Workspace-relative.
    pub path: String,
    pub line_number: u32,
    pub line_content: String,
    /// UTF-16 code-unit offsets into `line_content`, end exclusive.
    pub ranges: Vec<(u32, u32)>,
}

#[derive(Debug, Default)]
pub struct ContentSearch {
    pub matches: Vec<ContentMatch>,
    pub truncated: bool,
    /// Why the query is not a valid pattern; the search then ran it literally.
    pub regex_error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ContentSearchError {
    #[error("ripgrep is not installed in this sandbox")]
    NotInstalled,
    #[error("ripgrep failed: {0}")]
    Failed(String),
}

/// Up to `query.max_results` matching lines in `workspace_dir`.
///
/// An invalid pattern is searched as a literal instead, and says why in
/// [`ContentSearch::regex_error`], as the reference server does.
pub async fn search_contents(
    workspace_dir: &str,
    query: &ContentQuery<'_>,
) -> Result<ContentSearch, ContentSearchError> {
    if query.query.is_empty() {
        return Ok(ContentSearch::default());
    }
    let max = match query.max_results {
        n if n <= 0 => DEFAULT_MAX_RESULTS,
        n => (n as usize).min(HARD_MAX_RESULTS),
    };
    match ripgrep(workspace_dir, query, query.use_regex, max).await? {
        Outcome::Searched(search) => Ok(search),
        Outcome::BadPattern(reason) => match ripgrep(workspace_dir, query, false, max).await? {
            Outcome::Searched(search) => Ok(ContentSearch {
                regex_error: Some(reason),
                ..search
            }),
            Outcome::BadPattern(reason) => Err(ContentSearchError::Failed(reason)),
        },
    }
}

enum Outcome {
    Searched(ContentSearch),
    BadPattern(String),
}

async fn ripgrep(
    workspace_dir: &str,
    query: &ContentQuery<'_>,
    regex: bool,
    max: usize,
) -> Result<Outcome, ContentSearchError> {
    let mut command = Command::new("rg");
    command
        .current_dir(workspace_dir)
        .args(["--json", "--no-config", "--hidden"])
        .args(["--max-count", MAX_MATCHES_PER_FILE])
        .args(["--max-filesize", MAX_FILE_SIZE])
        .arg(if query.case_sensitive {
            "--case-sensitive"
        } else {
            "--ignore-case"
        });
    for dir in UNBROWSABLE_DIRS {
        command.arg("--glob").arg(format!("!{dir}"));
    }
    if query.whole_word {
        command.arg("--word-regexp");
    }
    if !regex {
        command.arg("--fixed-strings");
    }
    // An explicit path: given none and a non-terminal stdin, ripgrep searches
    // stdin instead of the directory.
    command
        .arg("--regexp")
        .arg(query.query)
        .args(["--", "."])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command.spawn().map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => ContentSearchError::NotInstalled,
        _ => ContentSearchError::Failed(error.to_string()),
    })?;
    let (Some(stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(ContentSearchError::Failed("no output pipes".into()));
    };
    // Drained beside stdout: a child blocked writing a full stderr pipe would
    // never close stdout.
    let stderr = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });

    let mut lines = BufReader::new(stdout).lines();
    let mut matches = Vec::new();
    let mut truncated = false;
    let deadline = tokio::time::sleep(SEARCH_DEADLINE);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    let Some(found) = parse_match(&line) else { continue };
                    if matches.len() == max {
                        truncated = true;
                        break;
                    }
                    matches.push(found);
                }
                Ok(None) => break,
                Err(error) => return Err(ContentSearchError::Failed(error.to_string())),
            },
            () = &mut deadline => {
                truncated = true;
                break;
            }
        }
    }
    if truncated {
        let _ = child.kill().await;
        stderr.abort();
        return Ok(Outcome::Searched(ContentSearch {
            matches,
            truncated,
            regex_error: None,
        }));
    }

    let status = child
        .wait()
        .await
        .map_err(|error| ContentSearchError::Failed(error.to_string()))?;
    let stderr = stderr.await.unwrap_or_default();
    // Exit 2 is any error, including an unreadable file met beside matches;
    // only a pattern ripgrep could not parse makes the search itself wrong.
    if status.code() == Some(2) {
        if stderr.contains("regex parse error") {
            return Ok(Outcome::BadPattern(stderr.trim().to_string()));
        }
        warn!(stderr = %stderr.trim(), "ripgrep reported errors during a content search");
    }
    Ok(Outcome::Searched(ContentSearch {
        matches,
        truncated,
        regex_error: None,
    }))
}

/// One `match` message of ripgrep's JSON output. A path or line that is not
/// UTF-8 arrives as base64 `bytes` and is skipped.
fn parse_match(line: &str) -> Option<ContentMatch> {
    let message: Value = serde_json::from_str(line).ok()?;
    if message.get("type")?.as_str()? != "match" {
        return None;
    }
    let data = message.get("data")?;
    let path = data.pointer("/path/text")?.as_str()?;
    let path = path.strip_prefix("./").unwrap_or(path);
    let text = data.pointer("/lines/text")?.as_str()?;
    let text = text
        .strip_suffix('\n')
        .map_or(text, |text| text.strip_suffix('\r').unwrap_or(text));
    let line_number = u32::try_from(data.get("line_number")?.as_u64()?).ok()?;

    let cut = text
        .char_indices()
        .nth(MAX_LINE_CHARS)
        .map_or(text.len(), |(index, _)| index);
    let line_content = &text[..cut];
    let ranges = data
        .get("submatches")?
        .as_array()?
        .iter()
        .filter_map(|submatch| {
            let start = usize::try_from(submatch.get("start")?.as_u64()?).ok()?;
            let end = usize::try_from(submatch.get("end")?.as_u64()?)
                .ok()?
                .min(cut);
            let on_boundaries =
                line_content.is_char_boundary(start) && line_content.is_char_boundary(end);
            (start < end && on_boundaries).then(|| {
                (
                    utf16_offset(line_content, start),
                    utf16_offset(line_content, end),
                )
            })
        })
        .collect();
    Some(ContentMatch {
        path: path.to_string(),
        line_number,
        line_content: line_content.to_string(),
        ranges,
    })
}

/// The UTF-16 offset of byte offset `byte` in `text`, which the client indexes
/// its strings by.
fn utf16_offset(text: &str, byte: usize) -> u32 {
    u32::try_from(text[..byte].encode_utf16().count()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn query(text: &str) -> ContentQuery<'_> {
        ContentQuery {
            query: text,
            case_sensitive: false,
            whole_word: false,
            use_regex: false,
            max_results: 0,
        }
    }

    #[test]
    fn a_match_s_ranges_count_utf16_units_not_bytes() {
        let line = r#"{"type":"match","data":{"path":{"text":"./src/a.ts"},"lines":{"text":"const 計画 = plan;\n"},"line_number":3,"submatches":[{"match":{"text":"plan"},"start":15,"end":19}]}}"#;
        assert_eq!(
            parse_match(line),
            Some(ContentMatch {
                path: "src/a.ts".into(),
                line_number: 3,
                line_content: "const 計画 = plan;".into(),
                ranges: vec![(11, 15)],
            })
        );
    }

    #[test]
    fn a_line_past_the_cap_is_cut_and_keeps_only_ranges_inside_it() {
        let text = format!("{}needle", "x".repeat(MAX_LINE_CHARS));
        let line = serde_json::json!({
            "type": "match",
            "data": {
                "path": {"text": "./long.txt"},
                "lines": {"text": text},
                "line_number": 1,
                "submatches": [{"start": MAX_LINE_CHARS, "end": MAX_LINE_CHARS + 6}],
            }
        })
        .to_string();
        let found = parse_match(&line).expect("a match");
        assert_eq!(found.line_content.chars().count(), MAX_LINE_CHARS);
        assert!(found.ranges.is_empty());
    }

    /// Against a real ripgrep, because what is under test is which files it
    /// reads and what it prints.
    mod against_ripgrep {
        use super::*;

        fn workspace() -> TempDir {
            let workspace = TempDir::new().expect("a temp dir");
            let repo = workspace.path().join("repo");
            std::fs::create_dir_all(repo.join(".git")).expect("a repository marker");
            std::fs::write(repo.join(".gitignore"), "build/\n").expect("ignore rules");
            std::fs::create_dir_all(repo.join("build")).expect("an ignored dir");
            std::fs::write(repo.join("build/out.js"), "let needle = 1;\n").expect("ignored");
            std::fs::write(
                repo.join("main.rs"),
                "fn main() {\n    let needle = 1;\n    let needles = 2;\n}\n",
            )
            .expect("a source file");
            workspace
        }

        fn root(workspace: &TempDir) -> &str {
            workspace.path().to_str().expect("a utf-8 temp path")
        }

        #[tokio::test]
        async fn matches_are_workspace_relative_and_skip_ignored_files() {
            let workspace = workspace();
            let found = search_contents(root(&workspace), &query("needle"))
                .await
                .expect("ripgrep ran");
            let lines: Vec<(&str, u32)> = found
                .matches
                .iter()
                .map(|found| (found.path.as_str(), found.line_number))
                .collect();
            assert_eq!(lines, vec![("repo/main.rs", 2), ("repo/main.rs", 3)]);
            assert_eq!(found.matches[0].ranges, vec![(8, 14)]);
            assert!(!found.truncated);
        }

        #[tokio::test]
        async fn a_whole_word_query_skips_a_longer_word() {
            let workspace = workspace();
            let found = search_contents(
                root(&workspace),
                &ContentQuery {
                    whole_word: true,
                    ..query("needle")
                },
            )
            .await
            .expect("ripgrep ran");
            assert_eq!(found.matches.len(), 1);
            assert_eq!(found.matches[0].line_number, 2);
        }

        #[tokio::test]
        async fn the_cap_truncates() {
            let workspace = workspace();
            let found = search_contents(
                root(&workspace),
                &ContentQuery {
                    max_results: 1,
                    ..query("needle")
                },
            )
            .await
            .expect("ripgrep ran");
            assert_eq!(found.matches.len(), 1);
            assert!(found.truncated);
        }

        #[tokio::test]
        async fn an_invalid_pattern_is_searched_literally_and_says_why() {
            let workspace = workspace();
            std::fs::write(workspace.path().join("repo/odd.txt"), "call(needle\n")
                .expect("a file holding the literal");
            let found = search_contents(
                root(&workspace),
                &ContentQuery {
                    use_regex: true,
                    ..query("(needle")
                },
            )
            .await
            .expect("ripgrep ran");
            assert!(found.regex_error.is_some());
            assert_eq!(found.matches.len(), 1);
            assert_eq!(found.matches[0].path, "repo/odd.txt");
        }
    }
}
