//! `moat artifact` — publish HTML pages from a task so they outlive its
//! sandbox, and list what has been published.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use moatless_api_schemas::artifact::MAX_PAGE_BYTES;

use crate::client::MoatClient;
use crate::config::Config;
use crate::output;

#[derive(Subcommand)]
pub enum ArtifactAction {
    /// Publish one self-contained HTML file as the next version of an
    /// artifact.
    ///
    /// Inline the CSS, scripts and images (as data URIs); only the page itself
    /// is uploaded. Publishing again with the same slug adds a version, and the
    /// link to the artifact always opens the latest one.
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
            task,
        } => {
            let task_id = task.map_or_else(|| cfg.require_task_id(), Ok)?;
            let (file_name, html) = read_page(&path)?;
            let resp = moat
                .api()
                .publish_artifact(&task_id, &title, slug.as_deref(), &file_name, html)
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                println!(
                    "Published \"{}\" v{}: {}",
                    resp.title, resp.version, resp.latest_url
                );
            }
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
                            a.updated_at,
                            a.latest_url,
                        ]
                    })
                    .collect();
                output::print_table(&["SLUG", "VERSION", "TITLE", "UPDATED", "URL"], &rows);
            }
        }
    }
    Ok(())
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
