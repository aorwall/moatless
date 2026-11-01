//! `moat artifact` — publish HTML pages from a task so they outlive its
//! sandbox, list and download what has been published, and read the records
//! its pages save.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use moatless_api_client::{ArtifactData, InlineArtifact, PageOptions};
use moatless_api_schemas::artifact::{MAX_PAGE_BYTES, WriteArtifactRecordRequest};

use crate::client::MoatClient;
use crate::config::Config;
use crate::output;

#[derive(Subcommand)]
pub enum ArtifactAction {
    /// Publish one self-contained HTML file as the next version of an
    /// artifact.
    ///
    /// Inline the CSS, scripts and images (as data URIs). Publishing again
    /// with the same slug adds a version, and the link opens the latest one.
    #[command(after_long_help = inline_help())]
    Publish {
        /// The `.html` or `.htm` file to publish
        path: PathBuf,
        /// Title shown in the task's artifact list
        #[arg(long)]
        title: String,
        /// Stable name that later publishes reuse; derived from the title when
        /// omitted
        #[arg(long)]
        slug: Option<String>,
        /// Show the page in the task's thread, with the thread's theme
        #[arg(long)]
        inline: bool,
        /// Inline frame height in CSS pixels, 80 to 2000; the page shrinks it
        /// to fit its content
        #[arg(long, requires = "inline")]
        height: Option<i32>,
        /// Let the page save records through `window.moatless.artifact`, as
        /// the reader viewing it
        #[arg(long)]
        data: bool,
        /// Queue a message to this task's agent for each record a reader saves
        #[arg(long, requires = "data")]
        notify: bool,
        /// JSON file of record rules: per key prefix, what a reader's page may
        /// do and which of its writes notify this task's agent. Replaces
        /// --notify
        #[arg(long, requires = "data", conflicts_with = "notify")]
        rules: Option<PathBuf>,
        /// Let the page read a record collection, as `<scope>:<name>` such as
        /// `loop:review-runs`, through `moatless.artifact.collection(name)`;
        /// repeatable. Only readers who reach the scope read it
        #[arg(long = "collection", value_name = "SCOPE:NAME")]
        collections: Vec<String>,
        /// Let only users who can see the task open the page and its records.
        /// Without it a new page is open to any signed-in user, even when the
        /// task is private, and a republished page keeps its visibility
        #[arg(long)]
        private: bool,
        /// Task to publish on; defaults to `$TASK_ID`
        #[arg(long)]
        task: Option<String>,
    },
    /// List a task's published artifacts, most recently updated first
    List {
        /// Task to list; defaults to `$TASK_ID`
        #[arg(long)]
        task: Option<String>,
    },
    /// Download a published version's page as it was published
    Get {
        /// Artifact id, as `moat artifact list` and its URLs show it
        artifact: String,
        /// Version to download; defaults to the latest
        #[arg(long)]
        version: Option<u32>,
        /// File to write; defaults to standard output
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// Read and write the records an artifact's page saves
    Data {
        #[command(subcommand)]
        action: DataAction,
    },
}

#[derive(Subcommand)]
pub enum DataAction {
    /// List an artifact's records as JSON, oldest change first
    List {
        /// The artifact's slug on the task, or its id
        artifact: String,
        /// Only records whose keys start with this, such as `comments/` for a list
        #[arg(long)]
        prefix: Option<String>,
        /// Only records changed after this RFC 3339 time
        #[arg(long)]
        since: Option<String>,
        /// Task the slug is on; defaults to `$TASK_ID`
        #[arg(long)]
        task: Option<String>,
    },
    /// Print the record saved under a key as JSON
    Get {
        /// The artifact's slug on the task, or its id
        artifact: String,
        /// The record's key
        key: String,
        /// Task the slug is on; defaults to `$TASK_ID`
        #[arg(long)]
        task: Option<String>,
    },
    /// Save a record under a key, such as a reply the page shows its reader
    Save {
        /// The artifact's slug on the task, or its id
        artifact: String,
        /// The record's key; a record already under it is replaced
        key: String,
        /// The record's JSON. Not named `json`: that id is the global `--json`
        /// flag, and clap panics reading this string as its bool.
        #[arg(value_name = "JSON")]
        record: String,
        /// Task the slug is on; defaults to `$TASK_ID`
        #[arg(long)]
        task: Option<String>,
    },
    /// Append an item to a list, saved under a new key `<list>/<id>`
    Append {
        /// The artifact's slug on the task, or its id
        artifact: String,
        /// The list's name, such as `comments`
        list: String,
        /// The item's JSON
        #[arg(value_name = "JSON")]
        record: String,
        /// Task the slug is on; defaults to `$TASK_ID`
        #[arg(long)]
        task: Option<String>,
    },
    /// Delete the record under a key
    Delete {
        /// The artifact's slug on the task, or its id
        artifact: String,
        /// The record's key
        key: String,
        /// Task the slug is on; defaults to `$TASK_ID`
        #[arg(long)]
        task: Option<String>,
    },
}

/// The rules for writing an `--inline` or `--data` page; the fallback for an
/// agent without the `artifacts` skill.
const INLINE_HELP: &str = "\
Inline pages (--inline)

The page is shown in the task's conversation in a borderless frame, on the
conversation's own background. The frame is as wide as the reply column (728px
on desktop by default, wider if the reader widens the chat, about 360px on
phones), and its left edge lines up with the reply text.

Layout:
- Leave html, body and the outermost element without a background colour. This
  overrides general style preferences such as a fixed dark page.
- Use a fluid width with no horizontal padding on the outermost element, and no
  outer card, border or banner title: the page is part of the reply.
- Give a box that needs its own background (a mock of a specific screen, a
  panel that must stand apart) at least 16px of padding on every side and
  var(--radius) corners.
- Give charts fixed pixel heights rather than heights that scale with width.
- Let the content set the page's height. Do not use 100vh, or height:100% on
  html or body: the frame grows to fit the page, so those make it grow again
  and again.

Height: the frame fits the page's height at each reader's width. --height <px>
(80 to 2000, default 1200) caps it, and content past the cap scrolls inside
the frame.

Theme: these CSS custom properties are set on :root and follow the reader's
theme and light/dark mode live:
{THEME_VARIABLES}
Use them rather than fixed colours. A base stylesheet sets the html background,
colour and font from them, sets the body margin to 0 and hides the page's
scrollbar; your own CSS overrides it. The page looks the same when it is opened
from its link.

Interactive pages (--data, --notify, --rules)

With --data the page is served with a `window.moatless.artifact` object that
reads and writes records as the reader viewing the page. A record is JSON
under a key; keys are `/`-separated, so a list is the keys under one prefix:
  await moatless.artifact.save(\"state\", state)    save under a key, replacing it
  await moatless.artifact.append(\"comments\", c)  save under a new comments/<id>
  await moatless.artifact.delete(\"comments/<id>\") remove a record
  await moatless.artifact.get(\"state\")            the record under a key, or null
  await moatless.artifact.list({prefix, since})   records, oldest change first
  moatless.artifact.subscribe({prefix}, onChange) calls onChange(records) on
                                                  every change; returns stop()
  moatless.artifact.viewer                        the reader's login
  moatless.artifact.viewerId                      the reader's user id
Records are JSON of at most 64 KiB, at most 10000 per artifact, and survive
republishing. The object is missing when the page is opened as a local file
or downloaded, so disable the page's controls without it.

With --notify each write a reader makes is also sent to this task's agent as
a message with <source>artifact</source> that names the record's key. While
it waits for a turn, the same reader's later writes join it as a list of
keys. Read records with `moat artifact data get <slug> <key>` or
`moat artifact data list <slug> [--prefix <list>/]`, and write one the page can
show with `moat artifact data save <slug> <key> '<json>'`. `moat messages reply`
on that message saves the reply as the record `agent-reply/<message id>`.
Records you write carry `writer: \"agent\"`; a page's carry `writer: \"page\"`.

With --rules <file> the server checks each page operation against per-prefix
rules, and only the writes a rule lists under notify reach this task's agent:
  {\"rulesVersion\": 1, \"prefixes\": {
    \"requests/\": {\"read\": \"anyone\", \"create\": \"anyone\", \"notify\": [\"create\"]},
    \"plans/\":    {\"read\": \"anyone\"},
    \"drafts/\":   {\"read\": \"owner\", \"create\": \"anyone\", \"update\": \"owner\", \"delete\": \"owner\"}}}
read, update and delete take \"anyone\" or \"owner\" (the reader who created the
record; an update keeps its owner); create takes \"anyone\". An operation left out is refused to pages,
so leaving out update and delete makes a record write-once. The longest
matching prefix decides, \"\" matches every key, and a key no prefix matches
is refused. Under an \"owner\" read rule a page lists its reader's own records
and every record you wrote. A refused write rejects with code
\"permission-denied\". Your own writes skip the rules. Each publish sets the
rules again, so pass --rules every time.

Record collections (--collection <scope>:<name>)

A collection is records kept in a scope rather than on one artifact, written
with `moat records save <name> <key> '<json>' --scope <scope>`, so the tasks
of one Loop or Workspace add to it over time. --collection loop:review-runs
lets the page read it, read-only and without --data:
  const runs = moatless.artifact.collection(\"review-runs\")
  await runs.list({prefix, since})   records, oldest change first
  await runs.get(key)                the record under a key, or null
  runs.subscribe({prefix}, onChange) calls onChange(records) on every change
  moatless.artifact.collections      [{scope, name}] the page may read
A reader who cannot reach the scope (another user's `user` scope, a Workspace
they cannot see) gets code \"permission-denied\". Each publish sets the
collections again, so pass --collection every time.";

/// The variables T3's bootstrap defines; `scripts/vendor-html-render-bootstrap.sh`
/// writes it from the same t3code source as the bootstrap.
const THEME_VARIABLES: &str = include_str!("html_render_theme_variables.txt");

fn inline_help() -> String {
    INLINE_HELP.replace("{THEME_VARIABLES}", THEME_VARIABLES.trim_end())
}

pub async fn run(
    moat: &MoatClient,
    action: ArtifactAction,
    cfg: &Config,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        ArtifactAction::Publish {
            path,
            title,
            slug,
            inline,
            height,
            data,
            notify,
            rules,
            collections,
            private,
            task,
        } => {
            let task_id = task.map_or_else(|| cfg.require_task_id(), Ok)?;
            let (file_name, html) = read_page(&path)?;
            let rules = rules
                .map(|path| {
                    std::fs::read_to_string(&path)
                        .map_err(|e| format!("cannot read {}: {e}", path.display()))
                })
                .transpose()?;
            let options = PageOptions {
                inline: inline.then_some(InlineArtifact { height }),
                data: ArtifactData {
                    accepts: data,
                    notify,
                    rules,
                    collections,
                },
                private,
            };
            let resp = moat
                .api()
                .publish_artifact(
                    &task_id,
                    &title,
                    slug.as_deref(),
                    &options,
                    &file_name,
                    html,
                )
                .await?;
            output::emit(json, &resp, || {
                println!(
                    "Published \"{}\" v{}: {}",
                    resp.title, resp.version, resp.latest_url
                );
            });
        }
        ArtifactAction::List { task } => {
            let task_id = task.map_or_else(|| cfg.require_task_id(), Ok)?;
            let resp = moat.api().list_task_artifacts(&task_id).await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                let rows: Vec<Vec<String>> = resp
                    .items
                    .into_iter()
                    .map(|a| {
                        vec![
                            a.slug,
                            format!("v{}", a.version),
                            a.title,
                            a.visibility.to_string(),
                            a.updated_at,
                            a.latest_url,
                        ]
                    })
                    .collect();
                output::print_table(
                    &["SLUG", "VERSION", "TITLE", "VISIBILITY", "UPDATED", "URL"],
                    &rows,
                );
            }
        }
        ArtifactAction::Get {
            artifact,
            version,
            output,
        } => {
            let html = moat
                .api()
                .download_artifact_page(&artifact, version)
                .await?;
            if let Some(path) = output {
                std::fs::write(&path, &html)
                    .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
                eprintln!("Wrote {} bytes to {}", html.len(), path.display());
            } else {
                use std::io::Write as _;
                std::io::stdout().write_all(&html)?;
            }
        }
        ArtifactAction::Data { action } => run_data(moat, action, cfg).await?,
    }
    Ok(())
}

/// Records print as JSON whatever `--json` says: they are JSON already.
async fn run_data(
    moat: &MoatClient,
    action: DataAction,
    cfg: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
    let (artifact, task, req) = match action {
        DataAction::List {
            artifact,
            prefix,
            since,
            task,
        } => {
            let artifact_id = resolve_artifact(moat, cfg, &artifact, task).await?;
            let resp = moat
                .api()
                .list_artifact_records(&artifact_id, prefix.as_deref(), since.as_deref())
                .await?;
            output::print_json_typed(&resp);
            return Ok(());
        }
        DataAction::Get {
            artifact,
            key,
            task,
        } => {
            let artifact_id = resolve_artifact(moat, cfg, &artifact, task).await?;
            let resp = moat
                .api()
                .list_artifact_records(&artifact_id, Some(&key), None)
                .await?;
            let record = resp
                .items
                .into_iter()
                .find(|r| r.key == key)
                .ok_or_else(|| format!("artifact {artifact} has no record under key {key:?}"))?;
            output::print_json_typed(&record);
            return Ok(());
        }
        DataAction::Save {
            artifact,
            key,
            record,
            task,
        } => {
            let data = parse_record(&record)?;
            (
                artifact,
                task,
                WriteArtifactRecordRequest::Save { key, data },
            )
        }
        DataAction::Append {
            artifact,
            list,
            record,
            task,
        } => {
            let data = parse_record(&record)?;
            (
                artifact,
                task,
                WriteArtifactRecordRequest::Append { list, data },
            )
        }
        DataAction::Delete {
            artifact,
            key,
            task,
        } => (artifact, task, WriteArtifactRecordRequest::Delete { key }),
    };
    let artifact_id = resolve_artifact(moat, cfg, &artifact, task).await?;
    let resp = moat.api().write_artifact_record(&artifact_id, &req).await?;
    output::print_json_typed(&resp);
    Ok(())
}

fn parse_record(record: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(record).map_err(|e| format!("the record is not JSON: {e}"))
}

/// The id of the artifact `name` names: a slug on the task, else an id.
async fn resolve_artifact(
    moat: &MoatClient,
    cfg: &Config,
    name: &str,
    task: Option<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    let Ok(task_id) = task.map_or_else(|| cfg.require_task_id(), Ok) else {
        return Ok(name.to_string());
    };
    let listed = moat.api().list_task_artifacts(&task_id).await?;
    Ok(listed
        .items
        .into_iter()
        .find(|a| a.slug == name)
        .map_or_else(|| name.to_string(), |a| a.artifact_id))
}

/// The page's file name and bytes. Refuses a directory, a non-HTML file and,
/// before reading it, a file over the server's limit.
fn read_page(path: &Path) -> Result<(String, Vec<u8>), Box<dyn std::error::Error>> {
    let meta =
        std::fs::metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!(
            "{} is not a file; an artifact is one self-contained HTML file",
            path.display()
        )
        .into());
    }
    let is_html = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"));
    if !is_html {
        return Err(format!("{} is not an .html or .htm file", path.display()).into());
    }
    if meta.len() > MAX_PAGE_BYTES as u64 {
        return Err(format!("{} exceeds {MAX_PAGE_BYTES} bytes", path.display()).into());
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{} has no UTF-8 file name", path.display()))?;
    Ok((name.to_string(), std::fs::read(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_html_file_is_published_under_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Report.HTML");
        std::fs::write(&file, "<p>hi</p>").unwrap();
        let (name, html) = read_page(&file).unwrap();
        assert_eq!(name, "Report.HTML");
        assert_eq!(html, b"<p>hi</p>");
    }

    #[test]
    fn a_directory_or_a_non_html_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let css = dir.path().join("style.css");
        std::fs::write(&css, "p{}").unwrap();
        assert!(read_page(dir.path()).is_err());
        assert!(read_page(&css).is_err());
    }

    /// A parser with the binary's global `--json`, which a positional with the
    /// same id breaks at runtime.
    #[derive(clap::Parser)]
    struct TestCli {
        #[arg(long, global = true)]
        json: bool,
        #[command(subcommand)]
        action: ArtifactAction,
    }

    #[test]
    fn data_save_takes_a_key_then_json_beside_the_global_json_flag() {
        use clap::Parser as _;
        let cli = TestCli::try_parse_from([
            "moat",
            "data",
            "save",
            "review",
            "reply",
            r#"{"a":1}"#,
            "--json",
        ])
        .expect("the arguments parse");
        assert!(cli.json);
        let ArtifactAction::Data {
            action: DataAction::Save { key, record, .. },
        } = cli.action
        else {
            panic!("expected data save");
        };
        assert_eq!((key.as_str(), record.as_str()), ("reply", r#"{"a":1}"#));
    }

    /// The prompt sends the agent to `--help` for these rules, and `-h` omits them.
    #[test]
    fn publish_long_help_carries_the_inline_rules() {
        use clap::Subcommand as _;
        let mut cmd = ArtifactAction::augment_subcommands(clap::Command::new("artifact"));
        let publish = cmd.find_subcommand_mut("publish").unwrap();
        let help = publish.render_long_help().to_string();
        assert!(help.contains("100vh"), "{help}");
        assert!(help.contains("--chart-1"), "{help}");
        assert!(!help.contains("{THEME_VARIABLES}"), "{help}");
        assert!(help.contains("moatless.artifact.save"), "{help}");
    }

    #[test]
    fn a_page_over_the_limit_is_refused_before_reading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.html");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_PAGE_BYTES as u64 + 1).unwrap();
        let err = read_page(&path).unwrap_err().to_string();
        assert!(err.contains("exceeds"), "{err}");
    }
}
