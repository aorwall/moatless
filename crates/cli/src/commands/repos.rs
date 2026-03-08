//! Repository commands: list, get, create, update.

use clap::Subcommand;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum RepoAction {
    /// List repositories
    List,
    /// Get repository details
    Get { repo_id: String },
    /// Create a repository
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        git_url: String,
        #[arg(long, default_value = "main")]
        branch: String,
        #[arg(long, default_value = "github")]
        provider: String,
        /// Repository kind: remote, template, or git
        #[arg(long, default_value = "remote")]
        kind: String,
    },
    /// Update a repository
    Update {
        repo_id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        git_url: Option<String>,
    },
}

pub async fn run(client: &MoatClient, action: RepoAction, json: bool) -> Result<(), ClientError> {
    match action {
        RepoAction::List => {
            let resp = client.get("/api/v1/repositories").await?;
            if json {
                output::print_json(&resp);
            } else {
                format_repo_list(&resp);
            }
        }
        RepoAction::Get { repo_id } => {
            let resp = client.get(&format!("/api/v1/repositories/{repo_id}")).await?;
            if json {
                output::print_json(&resp);
            } else {
                format_repo_detail(&resp);
            }
        }
        RepoAction::Create {
            name,
            git_url,
            branch,
            provider,
            kind,
        } => {
            let body = serde_json::json!({
                "name": name,
                "kind": kind,
                "remoteUrl": git_url,
                "defaultBranch": branch,
                "provider": provider,
            });
            let resp = client.post("/api/v1/repositories", &body).await?;
            if json {
                output::print_json(&resp);
            } else {
                let id = resp.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                println!("Repository created: {id}");
            }
        }
        RepoAction::Update {
            repo_id,
            name,
            branch,
            git_url,
        } => {
            let mut body = serde_json::Map::new();
            if let Some(n) = name {
                body.insert("name".into(), serde_json::json!(n));
            }
            if let Some(b) = branch {
                body.insert("defaultBranch".into(), serde_json::json!(b));
            }
            if let Some(u) = git_url {
                body.insert("remoteUrl".into(), serde_json::json!(u));
            }
            let resp = client
                .put(
                    &format!("/api/v1/repositories/{repo_id}"),
                    &serde_json::Value::Object(body),
                )
                .await?;
            if json {
                output::print_json(&resp);
            } else {
                println!("Repository {repo_id} updated.");
            }
        }
    }
    Ok(())
}

fn format_repo_list(resp: &serde_json::Value) {
    let items = resp.as_array().cloned().unwrap_or_default();

    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|r| {
            vec![
                r.get("id").and_then(|v| v.as_str()).unwrap_or("-").to_string(),
                r.get("name").and_then(|v| v.as_str()).unwrap_or("-").to_string(),
                r.get("provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
                r.get("defaultBranch")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
            ]
        })
        .collect();

    output::print_table(&["ID", "NAME", "PROVIDER", "BRANCH"], &rows);
}

fn format_repo_detail(resp: &serde_json::Value) {
    let id = resp.get("id").and_then(|v| v.as_str()).unwrap_or("-");
    let name = resp.get("name").and_then(|v| v.as_str()).unwrap_or("-");
    let provider = resp
        .get("gitProvider")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    let branch = resp
        .get("defaultBranch")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    let git_url = resp.get("remoteUrl").and_then(|v| v.as_str()).unwrap_or("-");

    println!("ID:       {id}");
    println!("Name:     {name}");
    println!("Provider: {provider}");
    println!("Branch:   {branch}");
    println!("Git URL:  {git_url}");
}
