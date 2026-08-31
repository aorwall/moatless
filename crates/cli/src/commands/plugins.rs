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
    /// One workspace, overriding both. Needs `--workspace`.
    Workspace,
}

impl From<Reach> for ActivationReach {
    fn from(r: Reach) -> Self {
        match r {
            Reach::Everyone => ActivationReach::Everyone,
            Reach::Personal => ActivationReach::Personal,
            Reach::Workspace => ActivationReach::Workspace,
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
        /// Which workspace, with `--reach workspace`
        #[arg(long)]
        workspace: Option<String>,
        /// Keep this record's say even where a workspace record disagrees.
        /// `--always=false` gives it up; omit to leave the record as it is
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
        always: Option<bool>,
        /// Frontload these skills into every fresh agent session instead of
        /// waiting to be picked for a turn. Omit to leave the record as it is
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
        frontload: Option<bool>,
        /// Whose personal record, with `--reach personal`. Naming somebody
        /// else is an administrator's call; omit for your own
        #[arg(long)]
        user: Option<uuid::Uuid>,
    },
    /// Turn a plugin, or one skill inside it, off
    Disable {
        plugin_id: String,
        #[arg(long, value_enum)]
        reach: Reach,
        /// Omit to mean the whole plugin
        #[arg(long)]
        skill: Option<String>,
        /// Which workspace, with `--reach workspace`
        #[arg(long)]
        workspace: Option<String>,
        /// Keep this record's say even where a workspace record disagrees —
        /// here, keeping the skills out
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
        always: Option<bool>,
        /// Frontloading, for whenever the record is turned back on
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
        frontload: Option<bool>,
        /// Whose personal record, with `--reach personal`. Naming somebody
        /// else is an administrator's call; omit for your own
        #[arg(long)]
        user: Option<uuid::Uuid>,
    },
    /// Remove a record, handing it back to whatever the next reach down says
    Clear {
        plugin_id: String,
        #[arg(long, value_enum)]
        reach: Reach,
        /// Omit to mean the whole-plugin record
        #[arg(long)]
        skill: Option<String>,
        /// Which workspace, with `--reach workspace`
        #[arg(long)]
        workspace: Option<String>,
        /// Whose personal record, with `--reach personal`. Naming somebody
        /// else is an administrator's call; omit for your own
        #[arg(long)]
        user: Option<uuid::Uuid>,
    },
    /// What somebody actually gets, once defaults and their own records resolve
    Effective {
        /// Resolve as it would be in this workspace
        #[arg(long)]
        workspace: Option<String>,
        /// Whose delivery to resolve — an administrator's call; omit for your
        /// own
        #[arg(long)]
        user: Option<uuid::Uuid>,
    },
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
            workspace,
            always,
            frontload,
            user,
        } => {
            let set = Set {
                reach,
                skill,
                workspace,
                user,
                always,
                frontload,
                enabled: true,
            };
            set_activation(client, &plugin_id, set, json).await?;
        }
        PluginAction::Disable {
            plugin_id,
            reach,
            skill,
            workspace,
            always,
            frontload,
            user,
        } => {
            let set = Set {
                reach,
                skill,
                workspace,
                user,
                always,
                frontload,
                enabled: false,
            };
            set_activation(client, &plugin_id, set, json).await?;
        }
        PluginAction::Clear {
            plugin_id,
            reach,
            skill,
            workspace,
            user,
        } => {
            client
                .api()
                .clear_plugin_activation(
                    &plugin_id,
                    reach.into(),
                    skill.as_deref(),
                    workspace.as_deref(),
                    user.map(|id| id.to_string()).as_deref(),
                )
                .await?;
            println!("Activation record cleared.");
        }
        PluginAction::Effective { workspace, user } => {
            let plugins = client
                .api()
                .list_effective_plugins(
                    workspace.as_deref(),
                    user.map(|id| id.to_string()).as_deref(),
                )
                .await?;
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
                            p.frontload_skill_names.join(", "),
                        ]
                    })
                    .collect();
                output::print_table(&["ID", "NAME", "SKILLS", "FRONTLOADED"], &rows);
            }
        }
    }
    Ok(())
}

/// One activation write, as the command line spells it. The two flags are
/// `None` when the command did not mention them, which leaves the stored record
/// alone rather than clearing what somebody else set.
struct Set {
    reach: Reach,
    skill: Option<String>,
    workspace: Option<String>,
    /// Whose personal record. `None` is the caller's own, which is the only
    /// one anybody but an administrator may write.
    user: Option<uuid::Uuid>,
    always: Option<bool>,
    frontload: Option<bool>,
    enabled: bool,
}

async fn set_activation(
    client: &MoatClient,
    plugin_id: &str,
    set: Set,
    json: bool,
) -> Result<(), ClientError> {
    let req = SetActivationRequest {
        reach: set.reach.into(),
        skill_name: set.skill,
        // `None` is the caller's own record, which is what `--reach personal`
        // reads as when nobody is named.
        user_id: set.user,
        workspace_id: set.workspace,
        enabled: set.enabled,
        always: set.always,
        frontload: set.frontload,
    };
    let named_user = req.user_id;
    let stored = client.api().set_plugin_activation(plugin_id, &req).await?;
    if json {
        output::print_json_typed(&stored);
    } else {
        let what = stored.skill_name.as_deref().unwrap_or("(whole plugin)");
        let state = if stored.enabled { "on" } else { "off" };
        // Whose record only when the command named somebody: on one's own it
        // adds an id the caller did not type and does not need.
        let whose = named_user
            .map(|id| format!(" (user {id})"))
            .unwrap_or_default();
        println!("{what} is now {state} for {:?}{whose}.", stored.reach);
        if stored.always {
            println!("  always: a workspace record cannot overturn this one.");
        }
        if stored.frontload {
            println!("  frontload: the body leads every fresh agent session.");
        }
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

#[cfg(test)]
mod tests {
    use super::PluginAction;

    /// `PluginAction` is a `Subcommand`, so parsing it standalone needs a
    /// `Parser` to hang it off.
    #[derive(clap::Parser)]
    struct TestCli {
        #[command(subcommand)]
        action: PluginAction,
    }

    /// `(always, frontload)` off a parsed `enable`.
    fn flags_of(extra: &[&str]) -> (Option<bool>, Option<bool>) {
        let mut argv = vec!["moat", "enable", "p1", "--reach", "everyone"];
        argv.extend_from_slice(extra);
        match <TestCli as clap::Parser>::try_parse_from(argv)
            .expect("parse")
            .action
        {
            PluginAction::Enable {
                always, frontload, ..
            } => (always, frontload),
            _ => unreachable!("the argv above only builds Enable"),
        }
    }

    #[test]
    fn a_personal_record_can_name_somebody_other_than_the_caller() {
        // Left off, the request carries no subject and the server reads it as
        // the caller — which is why this has to be an `Option`, not a default.
        let bare = <TestCli as clap::Parser>::try_parse_from([
            "moat", "enable", "p1", "--reach", "personal",
        ])
        .expect("parse")
        .action;
        let named = <TestCli as clap::Parser>::try_parse_from([
            "moat",
            "enable",
            "p1",
            "--reach",
            "personal",
            "--user",
            "00000000-0000-0000-0000-000000000001",
            "--frontload",
        ])
        .expect("parse")
        .action;

        match (bare, named) {
            (
                PluginAction::Enable { user: None, .. },
                PluginAction::Enable {
                    user: Some(id),
                    frontload,
                    ..
                },
            ) => {
                assert_eq!(id, uuid::Uuid::from_u128(1));
                assert_eq!(frontload, Some(true));
            }
            _ => unreachable!("the argv above only builds Enable"),
        }
    }

    #[test]
    fn a_user_that_is_not_a_uuid_is_refused_before_the_request() {
        assert!(
            <TestCli as clap::Parser>::try_parse_from([
                "moat", "enable", "p1", "--reach", "personal", "--user", "alice",
            ])
            .is_err(),
            "a login is not a user id; catching it here saves a round trip"
        );
    }

    #[test]
    fn a_flag_has_three_states_on_the_command_line() {
        // Unsaid is not the same as false: one leaves the stored record alone,
        // the other gives the flag up.
        assert_eq!(flags_of(&[]), (None, None));
        assert_eq!(flags_of(&["--always"]), (Some(true), None));
        assert_eq!(flags_of(&["--always=false"]), (Some(false), None));
        assert_eq!(
            flags_of(&["--always", "--frontload=true"]),
            (Some(true), Some(true))
        );
    }

    #[test]
    fn a_bare_flag_does_not_swallow_what_follows_it() {
        let mut argv = vec!["moat", "enable", "--always", "p1"];
        argv.extend_from_slice(&["--reach", "everyone"]);
        let action = <TestCli as clap::Parser>::try_parse_from(argv)
            .expect("parse")
            .action;
        match action {
            PluginAction::Enable {
                plugin_id, always, ..
            } => {
                assert_eq!(plugin_id, "p1", "the plugin id is not read as a flag value");
                assert_eq!(always, Some(true));
            }
            _ => unreachable!("the argv above only builds Enable"),
        }
    }
}
