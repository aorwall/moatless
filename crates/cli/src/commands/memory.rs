//! `moat memory` — keep what a task learns for later tasks, scoped to its
//! Loop, Workspace, repository, user or itself.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use moatless_api_client::MemoryListQuery;
use moatless_api_schemas::memory::{
    AppendMemoryRequest, ClaimMemoryRequest, MemoryEntryResponse, MemoryKeyRequest, MemoryScope,
    MemorySummaryResponse, PutMemoryRequest,
};

use crate::client::MoatClient;
use crate::config::Config;
use crate::output;

#[derive(Subcommand)]
pub enum MemoryAction {
    /// Create or update an entry. A new entry needs `--summary`.
    Put(Box<PutArgs>),
    /// Print entries in full and record the read. `prefix*` prints every
    /// match.
    Get {
        key: String,
        #[command(flatten)]
        target: Target,
    },
    /// List entry summaries, newest first
    List {
        #[arg(long)]
        prefix: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        /// Only entries updated within this long, e.g. `7d`
        #[arg(long, value_parser = parse_duration)]
        since: Option<i64>,
        #[arg(long)]
        limit: Option<i64>,
        #[command(flatten)]
        target: Target,
    },
    /// Search keys, summaries, tags and bodies; prints summaries, best match
    /// first
    Search {
        /// Words, "quoted phrases", -excluded, or
        query: String,
        #[arg(long)]
        limit: Option<i64>,
        #[command(flatten)]
        target: Target,
    },
    /// Add a dated line to an entry's log without rewriting it
    Append {
        key: String,
        #[arg(long)]
        text: String,
        #[command(flatten)]
        target: Target,
    },
    /// Print an entry's history, newest first
    Log {
        key: String,
        #[arg(long)]
        limit: Option<i64>,
        #[command(flatten)]
        target: Target,
    },
    /// Record that an entry's citations still hold, renewing its expiry
    Verify {
        key: String,
        #[command(flatten)]
        target: Target,
    },
    /// Claim an entry so no other open task works the same item; fails while
    /// another holds it
    Claim {
        key: String,
        /// How long the claim lasts unless released, e.g. `2h`
        #[arg(long, value_parser = parse_duration)]
        ttl: i64,
        #[command(flatten)]
        target: Target,
    },
    /// Drop this task's claim on an entry
    Release {
        key: String,
        #[command(flatten)]
        target: Target,
    },
    /// Delete an entry and its history
    Delete {
        key: String,
        #[command(flatten)]
        target: Target,
    },
}

/// The scope and the task a command acts for.
#[derive(Args)]
pub struct Target {
    /// task, loop, workspace, repository, user or global. Reads and writes
    /// reach this scope only; without it, the task's Loop, else the task.
    #[arg(long, value_parser = parse_scope)]
    scope: Option<MemoryScope>,
    /// Task to act for; defaults to `$TASK_ID`
    #[arg(long)]
    task: Option<String>,
}

#[derive(Args)]
pub struct PutArgs {
    key: String,
    /// One line, at most 160 characters; listings and preloads show only this
    #[arg(long)]
    summary: Option<String>,
    /// The entry's body
    #[arg(long, conflicts_with = "file")]
    text: Option<String>,
    /// Read the body from a file, or `-` for stdin
    #[arg(long)]
    file: Option<PathBuf>,
    /// Structured state as JSON
    #[arg(long)]
    data: Option<String>,
    /// Replaces the entry's tags; repeatable
    #[arg(long = "tag")]
    tags: Vec<String>,
    /// Where a fact came from: `path:line`, `linear:SP-1`, `task:<id>`, a URL.
    /// Replaces the entry's citations; repeatable
    #[arg(long = "cite")]
    citations: Vec<String>,
    /// Expire this long after the last put or verify, e.g. `28d` (the default)
    #[arg(long, value_parser = parse_duration, conflicts_with = "no_expiry")]
    ttl: Option<i64>,
    #[arg(long)]
    no_expiry: bool,
    /// Reads flag the entry stale this long after its last put or verify
    #[arg(long, value_parser = parse_duration)]
    stale_after: Option<i64>,
    /// Refuse unless the entry is at this version; 0 means it must be new
    #[arg(long)]
    if_version: Option<i32>,
    #[command(flatten)]
    target: Target,
}

pub async fn run(
    moat: &MoatClient,
    action: MemoryAction,
    cfg: &Config,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let api = moat.api();
    match action {
        MemoryAction::Put(args) => {
            let args = *args;
            let task_id = task_id(cfg, args.target.task)?;
            let body = match (args.text, args.file) {
                (Some(text), _) => Some(text),
                (None, Some(path)) => Some(read_body(&path)?),
                (None, None) => None,
            };
            let data = args
                .data
                .map(|raw| serde_json::from_str(&raw))
                .transpose()
                .map_err(|e| format!("--data is not JSON: {e}"))?;
            let req = PutMemoryRequest {
                key: args.key,
                scope: args.target.scope,
                summary: args.summary,
                body,
                data,
                tags: (!args.tags.is_empty()).then_some(args.tags),
                citations: (!args.citations.is_empty()).then_some(args.citations),
                ttl_seconds: args.ttl,
                no_expiry: args.no_expiry,
                stale_after_seconds: args.stale_after,
                if_version: args.if_version,
            };
            let entry = api.put_memory(&task_id, &req).await?;
            output::emit(json, &entry, || {
                println!("Saved {} ({}) v{}", entry.key, entry.scope, entry.version);
            });
        }
        MemoryAction::Get { key, target } => {
            let task_id = task_id(cfg, target.task)?;
            let resp = api.get_memory(&task_id, &key, target.scope).await?;
            output::emit(json, &resp, || {
                for (i, entry) in resp.items.iter().enumerate() {
                    if i > 0 {
                        println!();
                    }
                    print_entry(entry);
                }
            });
        }
        MemoryAction::List {
            prefix,
            tag,
            since,
            limit,
            target,
        } => {
            let task_id = task_id(cfg, target.task)?;
            let query = MemoryListQuery {
                scope: target.scope,
                prefix: prefix.as_deref(),
                tag: tag.as_deref(),
                updated_since_seconds: since,
                limit,
            };
            let resp = api.list_memory(&task_id, &query).await?;
            emit_summaries(json, &resp, &resp.items);
        }
        MemoryAction::Search {
            query,
            limit,
            target,
        } => {
            let task_id = task_id(cfg, target.task)?;
            let resp = api
                .search_memory(&task_id, &query, target.scope, limit)
                .await?;
            emit_summaries(json, &resp, &resp.items);
        }
        MemoryAction::Append { key, text, target } => {
            let task_id = task_id(cfg, target.task)?;
            let req = AppendMemoryRequest {
                key,
                scope: target.scope,
                text,
            };
            let entry = api.append_memory(&task_id, &req).await?;
            output::emit(json, &entry, || {
                println!("Appended to {} ({})", entry.key, entry.scope);
            });
        }
        MemoryAction::Log { key, limit, target } => {
            let task_id = task_id(cfg, target.task)?;
            let resp = api.memory_log(&task_id, &key, target.scope, limit).await?;
            output::emit(json, &resp, || {
                let rows: Vec<Vec<String>> = resp
                    .items
                    .iter()
                    .map(|e| {
                        vec![
                            e.created_at.clone(),
                            e.op.to_string(),
                            format!("v{}", e.version),
                            e.task_id.clone().unwrap_or_default(),
                            e.text.clone().unwrap_or_default(),
                        ]
                    })
                    .collect();
                output::print_table(&["AT", "OP", "VERSION", "TASK", "TEXT"], &rows);
            });
        }
        MemoryAction::Verify { key, target } => {
            let task_id = task_id(cfg, target.task)?;
            let req = MemoryKeyRequest {
                key,
                scope: target.scope,
            };
            let entry = api.verify_memory(&task_id, &req).await?;
            output::emit(json, &entry, || {
                println!(
                    "Verified {} ({}){}",
                    entry.key,
                    entry.scope,
                    entry
                        .expires_at
                        .as_deref()
                        .map(|at| format!(", expires {at}"))
                        .unwrap_or_default()
                );
            });
        }
        MemoryAction::Claim { key, ttl, target } => {
            let task_id = task_id(cfg, target.task)?;
            let req = ClaimMemoryRequest {
                key,
                scope: target.scope,
                ttl_seconds: ttl,
            };
            let entry = api.claim_memory(&task_id, &req).await?;
            output::emit(json, &entry, || {
                println!(
                    "Claimed {} until {}",
                    entry.key,
                    entry.claim_expires_at.as_deref().unwrap_or("?")
                );
            });
        }
        MemoryAction::Release { key, target } => {
            let task_id = task_id(cfg, target.task)?;
            let req = MemoryKeyRequest {
                key: key.clone(),
                scope: target.scope,
            };
            api.release_memory(&task_id, &req).await?;
            if !json {
                println!("Released {key}");
            }
        }
        MemoryAction::Delete { key, target } => {
            let task_id = task_id(cfg, target.task)?;
            api.delete_memory(&task_id, &key, target.scope).await?;
            if !json {
                println!("Deleted {key}");
            }
        }
    }
    Ok(())
}

fn task_id(cfg: &Config, task: Option<String>) -> Result<String, Box<dyn std::error::Error>> {
    task.map_or_else(|| cfg.require_task_id(), Ok)
}

fn read_body(path: &PathBuf) -> Result<String, Box<dyn std::error::Error>> {
    if path.as_os_str() == "-" {
        let mut body = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut body)?;
        return Ok(body);
    }
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()).into())
}

fn emit_summaries<T: serde::Serialize>(json: bool, resp: &T, items: &[MemorySummaryResponse]) {
    output::emit(json, resp, || {
        let rows: Vec<Vec<String>> = items
            .iter()
            .map(|s| {
                let mut flags = Vec::new();
                if s.stale {
                    flags.push("stale".to_string());
                }
                if let Some(holder) = &s.claimed_by_task_id {
                    flags.push(format!("claimed by {holder}"));
                }
                vec![
                    s.key.clone(),
                    s.scope.to_string(),
                    s.updated_at.get(..10).unwrap_or(&s.updated_at).to_string(),
                    flags.join(", "),
                    s.summary.clone(),
                ]
            })
            .collect();
        output::print_table(&["KEY", "SCOPE", "UPDATED", "FLAGS", "SUMMARY"], &rows);
    });
}

fn print_entry(entry: &MemoryEntryResponse) {
    println!("# {} ({}, v{})", entry.key, entry.scope, entry.version);
    println!("{}", entry.summary);
    println!("updated {}", entry.updated_at);
    if let Some(task) = &entry.created_by_task_id {
        let by_loop = entry
            .created_by_loop_id
            .as_deref()
            .map(|l| format!(" (loop {l})"))
            .unwrap_or_default();
        println!("created {} by task {task}{by_loop}", entry.created_at);
    }
    if entry.stale {
        println!("STALE: verify its citations before relying on it");
    }
    if let Some(holder) = &entry.claimed_by_task_id {
        println!("claimed by {holder}");
    }
    if !entry.tags.is_empty() {
        println!("tags: {}", entry.tags.join(", "));
    }
    for citation in &entry.citations {
        println!("cite: {citation}");
    }
    if !entry.body.is_empty() {
        println!("\n{}", entry.body);
    }
    if let Some(data) = &entry.data {
        println!("\ndata: {data}");
    }
}

fn parse_scope(raw: &str) -> Result<MemoryScope, String> {
    MemoryScope::try_from(raw).map_err(|_| {
        format!("`{raw}` is not a scope: task, loop, workspace, repository, user or global")
    })
}

/// `30m`, `2h`, `28d`, `4w`, or plain seconds.
fn parse_duration(raw: &str) -> Result<i64, String> {
    let raw = raw.trim();
    let (digits, unit) = raw.split_at(raw.find(|c: char| !c.is_ascii_digit()).unwrap_or(raw.len()));
    let n: i64 = digits
        .parse()
        .map_err(|_| format!("`{raw}` is not a duration such as 30m, 2h or 28d"))?;
    let unit_seconds = match unit {
        "" | "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        "w" => 7 * 24 * 60 * 60,
        _ => return Err(format!("`{raw}` has an unknown unit; use s, m, h, d or w")),
    };
    n.checked_mul(unit_seconds)
        .filter(|&s| s > 0)
        .ok_or_else(|| format!("`{raw}` is out of range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_parse_with_and_without_a_unit() {
        assert_eq!(parse_duration("90"), Ok(90));
        assert_eq!(parse_duration("30m"), Ok(1800));
        assert_eq!(parse_duration("2h"), Ok(7200));
        assert_eq!(parse_duration("28d"), Ok(28 * 86400));
        assert!(parse_duration("0").is_err());
        assert!(parse_duration("2y").is_err());
        assert!(parse_duration("h").is_err());
    }

    #[test]
    fn scopes_parse_by_wire_name() {
        assert_eq!(parse_scope("loop"), Ok(MemoryScope::Loop));
        assert!(parse_scope("team").is_err());
    }
}
