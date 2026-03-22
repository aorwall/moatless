//! Plugin repo commands: list, create, delete, link, unlink.

use clap::Subcommand;
use moatless_api_schemas::plugin_repo::{CreatePluginRepoRequest, PluginRepoResponse};

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum PluginAction {
    /// List all plugin repos
    List,
    /// Create a new plugin repo
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        git_url: String,
    },
    /// Delete a plugin repo
    Delete { plugin_id: String },
    /// Link a plugin to a repository
    Link {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        plugin: String,
    },
    /// Unlink a plugin from a repository
    Unlink {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        plugin: String,
    },
    /// List plugins linked to a repository
    RepoPlugins { repo_id: String },
}

pub async fn run(client: &MoatClient, action: PluginAction, json: bool) -> Result<(), ClientError> {
    match action {
        PluginAction::List => {
            let plugins = client.api().list_plugin_repos().await?;
            if json {
                output::print_json_typed(&plugins);
            } else {
                format_plugin_list(&plugins);
            }
        }
        PluginAction::Create { name, git_url } => {
            let req = CreatePluginRepoRequest { name, git_url };
            let created = client.api().create_plugin_repo(&req).await?;
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Plugin repo created: {}", created.id);
            }
        }
        PluginAction::Delete { plugin_id } => {
            client.api().delete_plugin_repo(&plugin_id).await?;
            println!("Plugin repo {plugin_id} deleted.");
        }
        PluginAction::Link { repo, plugin } => {
            client.api().link_plugin_to_repo(&repo, &plugin).await?;
            println!("Plugin {plugin} linked to repository {repo}.");
        }
        PluginAction::Unlink { repo, plugin } => {
            client
                .api()
                .unlink_plugin_from_repo(&repo, &plugin)
                .await?;
            println!("Plugin {plugin} unlinked from repository {repo}.");
        }
        PluginAction::RepoPlugins { repo_id } => {
            let plugins = client.api().list_repo_plugins(&repo_id).await?;
            if json {
                output::print_json_typed(&plugins);
            } else {
                format_plugin_list(&plugins);
            }
        }
    }
    Ok(())
}

fn format_plugin_list(plugins: &[PluginRepoResponse]) {
    let rows: Vec<Vec<String>> = plugins
        .iter()
        .map(|p| vec![p.id.clone(), p.name.clone(), p.git_url.clone()])
        .collect();

    output::print_table(&["ID", "NAME", "GIT_URL"], &rows);
}
