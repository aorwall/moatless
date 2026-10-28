//! `moat records` — keep structured records in a scope a task reaches, so the
//! tasks of one Loop or Workspace build up data over time, and artifact pages
//! published with `--collection` read it.

use clap::{Args, Subcommand};
use moatless_api_schemas::artifact::WriteArtifactRecordRequest;
use moatless_api_schemas::collection::MemoryScope;

use super::memory::parse_scope;
use crate::client::MoatClient;
use crate::config::Config;
use crate::output;

#[derive(Subcommand)]
pub enum RecordsAction {
    /// Save a record under a key, replacing the one there
    Save {
        /// The collection's name, such as `review-runs`
        collection: String,
        /// `/`-separated key, such as `runs/123/abc1234`
        key: String,
        /// The record's JSON. Not named `json`: that id is the global `--json`
        /// flag.
        #[arg(value_name = "JSON")]
        record: String,
        #[command(flatten)]
        target: Target,
    },
    /// Append an item to a list, saved under a new key `<list>/<id>`
    Append {
        collection: String,
        /// The list's name, such as `runs`
        list: String,
        #[arg(value_name = "JSON")]
        record: String,
        #[command(flatten)]
        target: Target,
    },
    /// Print the record under a key as JSON
    Get {
        collection: String,
        key: String,
        #[command(flatten)]
        target: Target,
    },
    /// List a collection's records as JSON, oldest change first
    List {
        collection: String,
        /// Only records whose keys start with this, such as `runs/123/`
        #[arg(long)]
        prefix: Option<String>,
        /// Only records changed after this RFC 3339 time
        #[arg(long)]
        since: Option<String>,
        #[command(flatten)]
        target: Target,
    },
    /// Delete the record under a key
    Delete {
        collection: String,
        key: String,
        #[command(flatten)]
        target: Target,
    },
    /// List the scope's collections
    Collections {
        #[command(flatten)]
        target: Target,
    },
}

/// The scope and the task a command acts for.
#[derive(Args)]
pub struct Target {
    /// task, loop, workspace, repository, user or global; without it, the
    /// task's Loop, else the task
    #[arg(long, value_parser = parse_scope)]
    scope: Option<MemoryScope>,
    /// Task to act for; defaults to `$TASK_ID`
    #[arg(long)]
    task: Option<String>,
}

/// Records print as JSON whatever `--json` says, except the collection list.
pub async fn run(
    moat: &MoatClient,
    action: RecordsAction,
    cfg: &Config,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let api = moat.api();
    let (collection, target, req) = match action {
        RecordsAction::Save {
            collection,
            key,
            record,
            target,
        } => {
            let data = parse_record(&record)?;
            (
                collection,
                target,
                WriteArtifactRecordRequest::Save { key, data },
            )
        }
        RecordsAction::Append {
            collection,
            list,
            record,
            target,
        } => {
            let data = parse_record(&record)?;
            (
                collection,
                target,
                WriteArtifactRecordRequest::Append { list, data },
            )
        }
        RecordsAction::Delete {
            collection,
            key,
            target,
        } => (
            collection,
            target,
            WriteArtifactRecordRequest::Delete { key },
        ),
        RecordsAction::Get {
            collection,
            key,
            target,
        } => {
            let task_id = task_id(cfg, target.task)?;
            let resp = api
                .list_collection_records(&task_id, &collection, target.scope, Some(&key), None)
                .await?;
            let record = resp
                .items
                .into_iter()
                .find(|r| r.key == key)
                .ok_or_else(|| {
                    format!("collection {collection} has no record under key {key:?}")
                })?;
            output::print_json_typed(&record);
            return Ok(());
        }
        RecordsAction::List {
            collection,
            prefix,
            since,
            target,
        } => {
            let task_id = task_id(cfg, target.task)?;
            let resp = api
                .list_collection_records(
                    &task_id,
                    &collection,
                    target.scope,
                    prefix.as_deref(),
                    since.as_deref(),
                )
                .await?;
            output::print_json_typed(&resp);
            return Ok(());
        }
        RecordsAction::Collections { target } => {
            let task_id = task_id(cfg, target.task)?;
            let resp = api.list_collections(&task_id, target.scope).await?;
            output::emit(json, &resp, || {
                let rows: Vec<Vec<String>> = resp
                    .items
                    .iter()
                    .map(|c| {
                        vec![
                            c.name.clone(),
                            c.scope.to_string(),
                            c.record_count.to_string(),
                            c.last_updated_at.clone().unwrap_or_default(),
                        ]
                    })
                    .collect();
                output::print_table(&["NAME", "SCOPE", "RECORDS", "UPDATED"], &rows);
            });
            return Ok(());
        }
    };
    let task_id = task_id(cfg, target.task)?;
    let resp = api
        .write_collection_record(&task_id, &collection, target.scope, &req)
        .await?;
    output::print_json_typed(&resp);
    Ok(())
}

fn task_id(cfg: &Config, task: Option<String>) -> Result<String, Box<dyn std::error::Error>> {
    task.map_or_else(|| cfg.require_task_id(), Ok)
}

fn parse_record(record: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(record).map_err(|e| format!("the record is not JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(clap::Parser)]
    struct TestCli {
        #[arg(long, global = true)]
        json: bool,
        #[command(subcommand)]
        action: RecordsAction,
    }

    #[test]
    fn save_takes_a_collection_key_and_json_with_a_scope() {
        use clap::Parser as _;
        let cli = TestCli::try_parse_from([
            "moat",
            "save",
            "review-runs",
            "runs/1/abc",
            r#"{"a":1}"#,
            "--scope",
            "loop",
            "--json",
        ])
        .expect("the arguments parse");
        let RecordsAction::Save {
            collection,
            key,
            target,
            ..
        } = cli.action
        else {
            panic!("expected save");
        };
        assert_eq!(
            (collection.as_str(), key.as_str(), target.scope),
            ("review-runs", "runs/1/abc", Some(MemoryScope::Loop))
        );
    }
}
