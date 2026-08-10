//! Repository commands: list, get, create, update.
//!
//! A repository is only a connection to a git repo — run-configuration lives on
//! the workspace (synced from the repo's `.moatless/`), so these commands set
//! only git-connection fields.

use clap::Subcommand;
use moatless_api_schemas::enums::{RepositoryKind, RepositoryProvider, Scope};
use moatless_api_schemas::repository::{
    CreateRepositoryRequest, RepositoryResponse, UpdateRepositoryRequest,
};

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
        /// Clone URL. HTTPS (https://github.com/org/repo) or SSH
        /// (git@host:org/repo.git or ssh://…). SSH remotes authenticate with an
        /// ssh_key secret materialized into the sandbox ~/.ssh — no provider token.
        /// Optional — a repository need not have a remote to clone (a template
        /// starter has none); the backend imposes no per-kind requirement.
        #[arg(long)]
        git_url: Option<String>,
        #[arg(long, default_value = "main")]
        branch: String,
        /// Hosting provider (github, gitlab, gitness). Ignored for SSH clone URLs,
        /// which authenticate via an ssh_key secret rather than a provider token.
        #[arg(long, default_value = "github")]
        provider: String,
        /// Repository kind: remote, template, or git
        #[arg(long, default_value = "remote")]
        kind: String,
        /// Repository description
        #[arg(long)]
        description: Option<String>,
        /// Scope: global or user
        #[arg(long)]
        scope: Option<String>,
    },
    /// Update a repository
    Update {
        repo_id: String,
        #[arg(long)]
        name: Option<String>,
        /// Change access scope: global or user
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        git_url: Option<String>,
        /// Repository description
        #[arg(long)]
        description: Option<String>,
    },
    /// Sync .moatless/ loops and workspaces from the remote repository.
    /// Warning: this is the explicit sync, so it also reclaims workspaces that
    /// were overridden from the UI/CLI, replacing their local changes.
    SyncConfig { repo_id: String },
    /// Delete a repository (soft-delete — marks as deleted, preserves history)
    Delete {
        repo_id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

pub async fn run(client: &MoatClient, action: RepoAction, json: bool) -> Result<(), ClientError> {
    match action {
        RepoAction::List => {
            let repos = client.api().list_repositories().await?;
            if json {
                output::print_json_typed(&repos);
            } else {
                format_repo_list(&repos);
            }
        }
        RepoAction::Get { repo_id } => {
            let repo = client.api().get_repository(&repo_id).await?;
            if json {
                output::print_json_typed(&repo);
            } else {
                format_repo_detail(&repo);
            }
        }
        RepoAction::Create {
            name,
            git_url,
            branch,
            provider,
            kind,
            description,
            scope,
        } => {
            let req = CreateRepositoryRequest {
                name,
                kind: parse_repo_kind(&kind),
                remote_url: git_url,
                default_branch: Some(branch),
                provider: parse_repo_provider(&provider),
                description,
                scope: scope.as_deref().and_then(parse_scope),
            };
            let created = client.api().create_repository(&req).await?;
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Repository created: {}", created.id);
            }
        }
        RepoAction::Update {
            repo_id,
            name,
            scope,
            branch,
            git_url,
            description,
        } => {
            let parsed_scope = match scope.as_deref() {
                Some(s) => match parse_scope(s) {
                    Some(sc) => Some(sc),
                    None => {
                        return Err(ClientError::Other(format!(
                            "Invalid --scope '{s}' (expected: global or user)"
                        )));
                    }
                },
                None => None,
            };
            let req = UpdateRepositoryRequest {
                name,
                scope: parsed_scope,
                default_branch: branch,
                remote_url: git_url,
                description,
            };
            let updated = client.api().update_repository(&repo_id, &req).await?;
            if json {
                output::print_json_typed(&updated);
            } else {
                println!("Repository {} updated.", updated.id);
            }
        }
        RepoAction::SyncConfig { repo_id } => {
            let result = client.api().sync_config(&repo_id).await?;
            if json {
                output::print_json_typed(&result);
            } else {
                let loops = &result.loops;
                println!("Config synced for repository {}.", result.repository.id);
                println!(
                    "Loops: {} created, {} updated, {} removed, {} overrides preserved.",
                    loops.created, loops.updated, loops.removed, loops.overrides_skipped
                );
                for skip in &loops.skipped {
                    println!("  skipped {}: {}", skip.loop_name, skip.reason);
                }
                for warning in &loops.warnings {
                    println!(
                        "  warning {} ({}): {}",
                        warning.loop_name, warning.field, warning.message
                    );
                }
                let workspaces = &result.workspaces;
                println!(
                    "Workspaces: {} created, {} updated, {} removed.",
                    workspaces.created, workspaces.updated, workspaces.removed
                );
                if workspaces.reclaimed > 0 {
                    println!(
                        "  warning: {} overridden workspace(s) reclaimed from git; local changes replaced.",
                        workspaces.reclaimed
                    );
                }
            }
        }
        RepoAction::Delete { repo_id, yes } => {
            let repo = client.api().get_repository(&repo_id).await?;
            let name = repo.name.clone();
            let id = repo.id.clone();

            if !yes {
                use std::io::{IsTerminal, Write};
                if !std::io::stdin().is_terminal() {
                    return Err(ClientError::Other(
                        "Refusing to delete without --yes in non-interactive mode".to_string(),
                    ));
                }
                print!("Delete repository '{name}' ({id})? This cannot be undone. [y/N] ");
                std::io::stdout()
                    .flush()
                    .map_err(|e| ClientError::Other(format!("Failed to flush stdout: {e}")))?;
                let mut line = String::new();
                std::io::stdin()
                    .read_line(&mut line)
                    .map_err(|e| ClientError::Other(format!("Failed to read stdin: {e}")))?;
                let answer = line.trim().to_lowercase();
                if answer != "y" && answer != "yes" {
                    return Err(ClientError::Other("Aborted.".to_string()));
                }
            }

            client.api().delete_repository(&id).await?;
            if json {
                output::print_json_typed(&serde_json::json!({
                    "id": id,
                    "deleted": true,
                }));
            } else {
                println!("Deleted repository {name} ({id})");
            }
        }
    }
    Ok(())
}

fn parse_repo_kind(s: &str) -> RepositoryKind {
    match s {
        "template" => RepositoryKind::Template,
        "git" => RepositoryKind::Git,
        _ => RepositoryKind::Remote,
    }
}

fn parse_repo_provider(s: &str) -> Option<RepositoryProvider> {
    match s {
        "github" => Some(RepositoryProvider::Github),
        "gitlab" => Some(RepositoryProvider::Gitlab),
        "gitness" => Some(RepositoryProvider::Gitness),
        _ => None,
    }
}

fn parse_scope(s: &str) -> Option<Scope> {
    match s {
        "global" => Some(Scope::Global),
        "user" => Some(Scope::User),
        _ => None,
    }
}

fn format_repo_list(repos: &[RepositoryResponse]) {
    let rows: Vec<Vec<String>> = repos
        .iter()
        .map(|r| {
            vec![
                r.id.clone(),
                r.name.clone(),
                r.provider
                    .as_ref()
                    .map_or("-".to_string(), ToString::to_string),
                r.default_branch.as_deref().unwrap_or("-").to_string(),
            ]
        })
        .collect();

    output::print_table(&["ID", "NAME", "PROVIDER", "BRANCH"], &rows);
}

fn format_repo_detail(r: &RepositoryResponse) {
    println!("ID:       {}", r.id);
    println!("Name:     {}", r.name);
    println!(
        "Provider: {}",
        r.provider
            .as_ref()
            .map_or("-".to_string(), ToString::to_string)
    );
    println!("Branch:   {}", r.default_branch.as_deref().unwrap_or("-"));
    println!("Git URL:  {}", r.remote_url.as_deref().unwrap_or("-"));
    if let Some(ref desc) = r.description {
        println!("Description: {desc}");
    }
    println!("Scope:      {}", r.scope);
    if let Some(ref owner) = r.owner_user_id {
        println!("Owner:      {owner}");
    }
}
