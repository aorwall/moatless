//! Repository commands: list, get, create, update.

use clap::Subcommand;
use moatless_api_client::types::{
    CreateRepositoryRequest, RepositoryKind, RepositoryProvider, RepositoryResponse,
    UpdateRepositoryRequest,
};

use crate::client::{map_api_error, ClientError, MoatClient};
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
            let resp = client
                .api()
                .list_repositories()
                .await
                .map_err(map_api_error)?;
            let repos = resp.into_inner();
            if json {
                output::print_json_typed(&repos);
            } else {
                format_repo_list(&repos);
            }
        }
        RepoAction::Get { repo_id } => {
            let resp = client
                .api()
                .get_repository(&repo_id)
                .await
                .map_err(map_api_error)?;
            let repo = resp.into_inner();
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
        } => {
            let req = CreateRepositoryRequest {
                name,
                kind: parse_repo_kind(&kind),
                remote_url: Some(git_url),
                default_branch: Some(branch),
                provider: parse_repo_provider(&provider),
                auto_reload: None,
                build_command: None,
                build_output_directory: None,
                description: None,
                docker_image: None,
                enable_inspector: None,
                env_vars: None,
                plugins: None,
                server_start_commands: None,
                servers: None,
                setup_commands: None,
                startup_timeout_seconds: None,
                scope: None,
                system_prompt: None,
                team_id: None,
                visibility: None,
                resources: None,
            };
            let resp = client
                .api()
                .create_repository(&req)
                .await
                .map_err(map_api_error)?;
            let created = resp.into_inner();
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Repository created: {}", created.id);
            }
        }
        RepoAction::Update {
            repo_id,
            name,
            branch,
            git_url,
        } => {
            let req = UpdateRepositoryRequest {
                name,
                default_branch: branch,
                remote_url: git_url,
                auto_reload: None,
                build_command: None,
                build_output_directory: None,
                description: None,
                docker_image: None,
                enable_inspector: None,
                env_vars: None,
                plugins: None,
                server_start_commands: None,
                servers: None,
                setup_commands: None,
                startup_timeout_seconds: None,
                system_prompt: None,
                visibility: None,
                resources: None,
            };
            let resp = client
                .api()
                .update_repository(&repo_id, &req)
                .await
                .map_err(map_api_error)?;
            let updated = resp.into_inner();
            if json {
                output::print_json_typed(&updated);
            } else {
                println!("Repository {} updated.", updated.id);
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
    println!(
        "Branch:   {}",
        r.default_branch.as_deref().unwrap_or("-")
    );
    println!(
        "Git URL:  {}",
        r.remote_url.as_deref().unwrap_or("-")
    );
}
