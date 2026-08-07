//! Plugin commands: list, create, delete, skills, enable, disable, clear, effective.

use clap::{Subcommand, ValueEnum};
use moatless_api_schemas::plugin::{
    ActivationReach, CreatePluginRequest, PluginResponse, SetActivationRequest,
};

use crate::client::{ClientError, MoatClient};
use crate::output;

/// Who a change applies to. Mirrors [`ActivationReach`]; kept separate so the
/// command line spells it the way `clap` wants without the wire type having to.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Reach {
    /// The default for everybody. Admins only.
    Everyone,
    /// Just you, overriding the default.
    Personal,
}

impl From<Reach> for ActivationReach {
    fn from(r: Reach) -> Self {
        match r {
            Reach::Everyone => ActivationReach::Everyone,
            Reach::Personal => ActivationReach::Personal,
        }
    }
}

#[derive(Subcommand)]
pub enum PluginAction {
    /// List all registered plugins
    List,
    /// Register a new plugin
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        git_url: String,
    },
    /// Delete a plugin
    Delete { plugin_id: String },
    /// List a plugin's skills, syncing from its source first
    Skills { plugin_id: String },
    /// Turn a plugin, or one skill inside it, on
    Enable {
        plugin_id: String,
        #[arg(long, value_enum)]
        reach: Reach,
        /// Omit to mean the whole plugin
        #[arg(long)]
        skill: Option<String>,
    },
    /// Turn a plugin, or one skill inside it, off
    Disable {
        plugin_id: String,
        #[arg(long, value_enum)]
        reach: Reach,
        /// Omit to mean the whole plugin
        #[arg(long)]
        skill: Option<String>,
    },
    /// Remove a record, handing it back to whatever the default says
    Clear {
        plugin_id: String,
        #[arg(long, value_enum)]
        reach: Reach,
        /// Omit to mean the whole-plugin record
        #[arg(long)]
        skill: Option<String>,
    },
    /// What you actually get, once defaults and your own records resolve
    Effective,
}

pub async fn run(client: &MoatClient, action: PluginAction, json: bool) -> Result<(), ClientError> {
    match action {
        PluginAction::List => {
            let plugins = client.api().list_plugins().await?;
            if json {
                output::print_json_typed(&plugins);
            } else {
                format_plugin_list(&plugins);
            }
        }
        PluginAction::Create { name, git_url } => {
            let req = CreatePluginRequest { name, git_url };
            let created = client.api().create_plugin(&req).await?;
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Plugin created: {}", created.id);
            }
        }
        PluginAction::Delete { plugin_id } => {
            client.api().delete_plugin(&plugin_id).await?;
            println!("Plugin {plugin_id} deleted.");
        }
        PluginAction::Skills { plugin_id } => {
            let skills = client.api().list_plugin_skills(&plugin_id).await?;
            if json {
                output::print_json_typed(&skills);
            } else {
                let rows: Vec<Vec<String>> = skills
                    .iter()
                    .map(|s| vec![s.name.clone(), s.description.clone()])
                    .collect();
                output::print_table(&["NAME", "DESCRIPTION"], &rows);
            }
        }
        PluginAction::Enable {
            plugin_id,
            reach,
            skill,
        } => {
            set_activation(client, &plugin_id, reach, skill, true, json).await?;
        }
        PluginAction::Disable {
            plugin_id,
            reach,
            skill,
        } => {
            set_activation(client, &plugin_id, reach, skill, false, json).await?;
        }
        PluginAction::Clear {
            plugin_id,
            reach,
            skill,
        } => {
            client
                .api()
                .clear_plugin_activation(&plugin_id, reach.into(), skill.as_deref())
                .await?;
            println!("Activation record cleared.");
        }
        PluginAction::Effective => {
            let plugins = client.api().list_effective_plugins().await?;
            if json {
                output::print_json_typed(&plugins);
            } else {
                let rows: Vec<Vec<String>> = plugins
                    .iter()
                    .map(|p| {
                        vec![
                            p.plugin_id.clone(),
                            p.plugin_name.clone(),
                            p.skill_names.join(", "),
                        ]
                    })
                    .collect();
                output::print_table(&["ID", "NAME", "SKILLS"], &rows);
            }
        }
    }
    Ok(())
}

async fn set_activation(
    client: &MoatClient,
    plugin_id: &str,
    reach: Reach,
    skill_name: Option<String>,
    enabled: bool,
    json: bool,
) -> Result<(), ClientError> {
    let req = SetActivationRequest {
        reach: reach.into(),
        skill_name,
        // Always the caller's own record: the command line has no way to name
        // another user, and `--reach personal` reads as "me" here.
        user_id: None,
        enabled,
    };
    let stored = client.api().set_plugin_activation(plugin_id, &req).await?;
    if json {
        output::print_json_typed(&stored);
    } else {
        let what = stored.skill_name.as_deref().unwrap_or("(whole plugin)");
        let state = if stored.enabled { "on" } else { "off" };
        println!("{what} is now {state} for {:?}.", stored.reach);
    }
    Ok(())
}

fn format_plugin_list(plugins: &[PluginResponse]) {
    let rows: Vec<Vec<String>> = plugins
        .iter()
        .map(|p| {
            vec![
                p.id.clone(),
                p.name.clone(),
                p.git_url.clone(),
                p.synced_version.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();

    output::print_table(&["ID", "NAME", "GIT_URL", "SYNCED"], &rows);
}
