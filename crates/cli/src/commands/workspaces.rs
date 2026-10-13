//! Workspace commands: list, get, create, update, override, reset-git, delete.

use clap::Subcommand;
use clap::builder::PossibleValuesParser;
use moatless_api_schemas::enums::Scope;
use moatless_workspace_types::schemas::workspace::{
    CreateWorkspaceRequest, ScriptConfig, UpdateWorkspaceRequest, WorkspaceRepoInput,
    WorkspaceResponse,
};
use moatless_workspace_types::workspace::WorkspaceCredentialProvider;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum WorkspaceAction {
    /// List workspaces
    List,
    /// Get workspace details
    Get { workspace_id: String },
    /// Create a workspace
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        description: Option<String>,
        /// Access scope: global or user
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        docker_image: Option<String>,
        /// Allow docker-in-docker access in the sandbox
        #[arg(long)]
        docker_access: Option<bool>,
        #[arg(long)]
        sandbox_service_account: Option<String>,
        #[arg(long)]
        system_prompt: Option<String>,
        /// Setup commands as a JSON array, e.g. `["npm install","npm run
        /// build"]`
        #[arg(long)]
        setup_commands_json: Option<String>,
        /// Environment variables as a JSON object, e.g. `{"FOO":"bar"}`
        #[arg(long)]
        env_vars_json: Option<String>,
        /// Script configs as a JSON array, e.g. `[{"name":"test","command":"bun
        /// test"}]`
        #[arg(long)]
        scripts_json: Option<String>,
        /// Resources config as a JSON object
        #[arg(long)]
        resources_json: Option<String>,
        #[arg(long)]
        network_profile: Option<String>,
        /// Extra node labels for this workspace's sandbox pods, as a JSON
        /// object, e.g. `{"workload":"moatless-od"}`. Admin only. Pass
        /// `{}` to clear it.
        #[arg(long)]
        node_selector_json: Option<String>,
        /// Identity every task on this workspace acts as on a git host. Admin
        /// only. Pass an empty string to clear it.
        #[arg(long)]
        run_as_user_id: Option<String>,
        /// Repository placements as a JSON array of {repositoryId,...} objects
        #[arg(long)]
        repos_json: Option<String>,
    },
    /// Update a workspace (manual workspaces only; use `override` for
    /// git-synced)
    Update {
        workspace_id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Change access scope: global or user
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        docker_image: Option<String>,
        #[arg(long)]
        docker_access: Option<bool>,
        #[arg(long)]
        sandbox_service_account: Option<String>,
        #[arg(long)]
        system_prompt: Option<String>,
        #[arg(long)]
        setup_commands_json: Option<String>,
        #[arg(long)]
        env_vars_json: Option<String>,
        #[arg(long)]
        scripts_json: Option<String>,
        #[arg(long)]
        resources_json: Option<String>,
        #[arg(long)]
        network_profile: Option<String>,
        /// Extra node labels for this workspace's sandbox pods, as a JSON
        /// object, e.g. `{"workload":"moatless-od"}`. Admin only. Pass
        /// `{}` to clear it.
        #[arg(long)]
        node_selector_json: Option<String>,
        /// Identity every task on this workspace acts as on a git host. Admin
        /// only. Pass an empty string to clear it.
        #[arg(long)]
        run_as_user_id: Option<String>,
    },
    /// Override a git-synced workspace. Sets source=manual so git sync skips it
    /// going forward. Provenance fields are kept so the origin is visible. Use
    /// `reset-git` to re-enable git sync.
    Override {
        workspace_id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        docker_image: Option<String>,
        #[arg(long)]
        docker_access: Option<bool>,
        #[arg(long)]
        sandbox_service_account: Option<String>,
        #[arg(long)]
        system_prompt: Option<String>,
        #[arg(long)]
        setup_commands_json: Option<String>,
        #[arg(long)]
        env_vars_json: Option<String>,
        #[arg(long)]
        scripts_json: Option<String>,
        #[arg(long)]
        resources_json: Option<String>,
        #[arg(long)]
        network_profile: Option<String>,
        /// Extra node labels for this workspace's sandbox pods, as a JSON
        /// object, e.g. `{"workload":"moatless-od"}`. Admin only. Pass
        /// `{}` to clear it.
        #[arg(long)]
        node_selector_json: Option<String>,
        /// Identity every task on this workspace acts as on a git host. Admin
        /// only. Pass an empty string to clear it.
        #[arg(long)]
        run_as_user_id: Option<String>,
    },
    /// Reset an overridden workspace back to git-managed. Re-syncs immediately
    /// from the repository — warning: local changes to this workspace,
    /// including its repo placements, are replaced by the committed config.
    #[command(name = "reset-git")]
    ResetGit { workspace_id: String },
    /// Delete a workspace (soft-delete; manual workspaces only)
    Delete {
        workspace_id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// Manage provider credentials pinned to a workspace.
    ///
    /// A workspace credential (e.g. a GitHub PAT) overrides the run-as user's
    /// own for every task on it — sandbox clone/push and backend PR/repo-access
    /// calls alike — reaching repos that user's own token cannot (e.g. outside
    /// its org).
    Credentials {
        #[command(subcommand)]
        action: CredentialAction,
    },
    /// Declare a workspace's scripts; each becomes a Run button in the chat
    /// header's Project actions group, or is runnable via `moat tasks scripts
    /// run`.
    ///
    /// This only edits the declaration — nothing runs until `moat tasks scripts
    /// run` addresses it against a task's sandbox. A script with `--port` also
    /// gets a live preview tab, like a declared server.
    Scripts {
        #[command(subcommand)]
        action: WorkspaceScriptAction,
    },
}

#[derive(Subcommand)]
pub enum WorkspaceScriptAction {
    /// List the workspace's declared scripts
    List { workspace_id: String },
    /// Declare a new script
    Add {
        workspace_id: String,
        /// Script id — what `moat tasks scripts run` and the wire address it
        /// by. Must be unique on the workspace
        name: String,
        /// The script's shell command. Pass "-" to write a multi-line script on
        /// stdin instead of the command line — anything that would need
        /// escaping as a single argument (quotes, &&, a heredoc) belongs there
        #[arg(long)]
        command: String,
        /// Display label, if it should read differently from the id
        #[arg(long)]
        label: Option<String>,
        /// Port the script serves on, 1-65535 — setting this makes it
        /// previewable.
        ///
        /// As soon as the script starts, the UI publishes the port and opens a
        /// live preview tab automatically; no separate server declaration
        /// needed
        #[arg(long)]
        port: Option<u16>,
        /// Icon shown next to the script in the Run menu
        #[arg(long)]
        icon: Option<String>,
        /// Start the script automatically whenever a task's sandbox starts,
        /// instead of only when someone runs it. It still waits for the
        /// workspace's setup commands to finish first
        #[arg(long)]
        run_on_start: bool,
    },
    /// Change an existing script's command, port, label, icon, or start
    /// behavior
    Update {
        workspace_id: String,
        name: String,
        /// Replaces the whole command. Pass "-" to read a multi-line script
        /// from stdin (see `add --help`)
        #[arg(long)]
        command: Option<String>,
        /// Pass an empty string to clear a previously set label
        #[arg(long)]
        label: Option<String>,
        /// A port from 1-65535. Pass -1 to stop serving a preview and clear a
        /// previously set port. Use `--port=-1` (with the `=`) — `--port -1` is
        /// parsed as two separate flags
        #[arg(long, allow_hyphen_values = true)]
        port: Option<i32>,
        /// Pass an empty string to clear a previously set icon
        #[arg(long)]
        icon: Option<String>,
        /// Start the script automatically whenever a task's sandbox starts.
        /// Pass `--run-on-start=false` to go back to running it on request
        #[arg(long, num_args = 0..=1, default_missing_value = "true")]
        run_on_start: Option<bool>,
    },
    /// Remove a declared script
    Remove { workspace_id: String, name: String },
}

#[derive(Subcommand)]
pub enum CredentialAction {
    /// List the credentials pinned to a workspace (metadata only; never values)
    List { workspace_id: String },
    /// Pin a provider credential to a workspace, replacing any current value
    Set {
        workspace_id: String,
        /// The provider this credential authenticates against
        #[arg(value_parser = provider_values())]
        provider: String,
        /// Credential value (use "-" to read from stdin)
        #[arg(long, short = 'v')]
        value: Option<String>,
        /// Read the value from a file instead of --value
        #[arg(long, conflicts_with = "value")]
        value_file: Option<String>,
    },
    /// Remove a pinned credential; the workspace falls back to the run-as user
    Remove {
        workspace_id: String,
        /// The provider this credential authenticates against
        #[arg(value_parser = provider_values())]
        provider: String,
    },
}

/// The provider names `--help` offers, read from the enum the backend accepts,
/// so a provider added there cannot leave this list stale.
fn provider_values() -> PossibleValuesParser {
    PossibleValuesParser::new(WorkspaceCredentialProvider::ALL.iter().map(|p| p.as_str()))
}

pub async fn run(
    client: &MoatClient,
    action: WorkspaceAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        WorkspaceAction::List => {
            let workspaces = client.api().list_workspaces().await?;
            output::emit(json, &workspaces, || format_workspace_list(&workspaces));
        }
        WorkspaceAction::Get { workspace_id } => {
            let ws = client.api().get_workspace(&workspace_id).await?;
            output::emit(json, &ws, || format_workspace_detail(&ws));
        }
        WorkspaceAction::Create {
            name,
            description,
            scope,
            docker_image,
            docker_access,
            sandbox_service_account,
            system_prompt,
            setup_commands_json,
            env_vars_json,
            scripts_json,
            resources_json,
            network_profile,
            node_selector_json,
            run_as_user_id,
            repos_json,
        } => {
            let repos: Vec<WorkspaceRepoInput> = match repos_json.as_deref() {
                Some(s) => serde_json::from_str(s)
                    .map_err(|e| ClientError::Other(format!("Invalid --repos-json: {e}")))?,
                None => vec![],
            };
            let req = CreateWorkspaceRequest {
                name,
                description,
                // Presentation and task defaults are set from the settings UI;
                // this command has no flags for them.
                icon: None,
                default_model: None,
                default_effort: None,
                scope: scope.as_deref().and_then(parse_scope),
                docker_image,
                docker_access,
                sandbox_service_account,
                system_prompt,
                setup_commands: parse_json_opt(&setup_commands_json, "--setup-commands-json")?,
                env_vars: parse_json_opt(&env_vars_json, "--env-vars-json")?,
                scripts: parse_json_opt(&scripts_json, "--scripts-json")?,
                resources: parse_json_opt(&resources_json, "--resources-json")?,
                network_profile,
                node_selector: parse_json_opt(&node_selector_json, "--node-selector-json")?,
                run_as_user_id,
                repos,
            };
            let created = client.api().create_workspace(&req).await?;
            output::emit(json, &created, || {
                println!("Workspace created: {}", created.id);
            });
        }
        WorkspaceAction::Update {
            workspace_id,
            name,
            description,
            scope,
            docker_image,
            docker_access,
            sandbox_service_account,
            system_prompt,
            setup_commands_json,
            env_vars_json,
            scripts_json,
            resources_json,
            network_profile,
            node_selector_json,
            run_as_user_id,
        } => {
            let parsed_scope = parse_scope_arg(scope.as_deref())?;
            let req = UpdateWorkspaceRequest {
                name,
                description,
                // See the create arm above: absent leaves each one unchanged.
                icon: None,
                default_model: None,
                default_effort: None,
                scope: parsed_scope,
                docker_image,
                docker_access,
                sandbox_service_account,
                system_prompt,
                setup_commands: parse_json_opt(&setup_commands_json, "--setup-commands-json")?,
                env_vars: parse_json_opt(&env_vars_json, "--env-vars-json")?,
                scripts: parse_json_opt(&scripts_json, "--scripts-json")?,
                resources: parse_json_opt(&resources_json, "--resources-json")?,
                network_profile,
                node_selector: parse_json_opt(&node_selector_json, "--node-selector-json")?,
                run_as_user_id,
            };
            let updated = client.api().update_workspace(&workspace_id, &req).await?;
            output::emit(json, &updated, || {
                println!("Workspace {} updated.", updated.id);
            });
        }
        WorkspaceAction::Override {
            workspace_id,
            name,
            description,
            docker_image,
            docker_access,
            sandbox_service_account,
            system_prompt,
            setup_commands_json,
            env_vars_json,
            scripts_json,
            resources_json,
            network_profile,
            node_selector_json,
            run_as_user_id,
        } => {
            let req = UpdateWorkspaceRequest {
                name,
                description,
                // See the create arm above.
                icon: None,
                default_model: None,
                default_effort: None,
                scope: None,
                docker_image,
                docker_access,
                sandbox_service_account,
                system_prompt,
                setup_commands: parse_json_opt(&setup_commands_json, "--setup-commands-json")?,
                env_vars: parse_json_opt(&env_vars_json, "--env-vars-json")?,
                scripts: parse_json_opt(&scripts_json, "--scripts-json")?,
                resources: parse_json_opt(&resources_json, "--resources-json")?,
                network_profile,
                node_selector: parse_json_opt(&node_selector_json, "--node-selector-json")?,
                run_as_user_id,
            };
            let updated = client.api().override_workspace(&workspace_id, &req).await?;
            output::emit(json, &updated, || {
                println!(
                    "Workspace {} overridden (source=manual). Git sync will skip it until reset.",
                    updated.id
                );
                if let Some(ref path) = updated.source_config_path {
                    println!("  Originally synced from: {path}");
                }
                if let Some(ref repo_id) = updated.source_repository_id {
                    println!("  Source repository: {repo_id}");
                }
            });
        }
        WorkspaceAction::ResetGit { workspace_id } => {
            let result = client.api().reset_git_workspace(&workspace_id).await?;
            if json {
                output::print_json_typed(&result);
            } else if result.resynced {
                println!(
                    "Workspace {} reset to git-managed; local changes replaced by {}.",
                    result.workspace.id,
                    result
                        .workspace
                        .source_config_path
                        .as_deref()
                        .unwrap_or("the repository config")
                );
            } else {
                println!(
                    "Workspace {} left overridden: {}",
                    result.workspace.id,
                    result.error.as_deref().unwrap_or("re-sync did not run")
                );
            }
        }
        WorkspaceAction::Delete { workspace_id, yes } => {
            let ws = client.api().get_workspace(&workspace_id).await?;
            let name = ws.name.clone();
            let id = ws.id.clone();

            if !yes {
                use std::io::{IsTerminal, Write};
                if !std::io::stdin().is_terminal() {
                    return Err(ClientError::Other(
                        "Refusing to delete without --yes in non-interactive mode".to_string(),
                    ));
                }
                print!("Delete workspace '{name}' ({id})? This cannot be undone. [y/N] ");
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

            client.api().delete_workspace(&id).await?;
            output::emit(
                json,
                &serde_json::json!({
                    "id": id,
                    "deleted": true,
                }),
                || println!("Deleted workspace {name} ({id})"),
            );
        }
        WorkspaceAction::Credentials { action } => {
            run_credentials(client, action, json).await?;
        }
        WorkspaceAction::Scripts { action } => {
            run_workspace_scripts(client, action, json).await?;
        }
    }
    Ok(())
}

async fn run_workspace_scripts(
    client: &MoatClient,
    action: WorkspaceScriptAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        WorkspaceScriptAction::List { workspace_id } => {
            let ws = client.api().get_workspace(&workspace_id).await?;
            let scripts = ws.scripts.unwrap_or_default();
            if json {
                output::print_json_typed(&scripts);
            } else if scripts.is_empty() {
                println!("No scripts declared on workspace {workspace_id}.");
            } else {
                format_scripts_list(&scripts);
            }
        }
        WorkspaceScriptAction::Add {
            workspace_id,
            name,
            command,
            label,
            port,
            icon,
            run_on_start,
        } => {
            if let Some(port) = port {
                validate_port(i32::from(port))?;
            }
            let command = read_value(Some(command))?.unwrap_or_default();
            let mut scripts = client
                .api()
                .get_workspace(&workspace_id)
                .await?
                .scripts
                .unwrap_or_default();
            if scripts.iter().any(|s| s.name == name) {
                return Err(ClientError::Other(format!(
                    "A script named \"{name}\" already exists. Use `update` to change it."
                )));
            }
            scripts.push(ScriptConfig {
                name,
                label,
                command,
                icon,
                port,
                env_vars: None,
                run_on_start,
            });
            let updated = replace_scripts(client, &workspace_id, scripts).await?;
            print_scripts_result(&updated, json);
        }
        WorkspaceScriptAction::Update {
            workspace_id,
            name,
            command,
            label,
            port,
            icon,
            run_on_start,
        } => {
            let mut scripts = client
                .api()
                .get_workspace(&workspace_id)
                .await?
                .scripts
                .unwrap_or_default();
            let script = scripts
                .iter_mut()
                .find(|s| s.name == name)
                .ok_or_else(|| not_declared(&workspace_id, &name))?;
            if let Some(command) = command {
                script.command = read_value(Some(command))?.unwrap_or_default();
            }
            if let Some(label) = label {
                script.label = (!label.is_empty()).then_some(label);
            }
            if let Some(icon) = icon {
                script.icon = (!icon.is_empty()).then_some(icon);
            }
            if let Some(port) = port {
                script.port = if port < 0 {
                    None
                } else {
                    Some(validate_port(port)?)
                };
            }
            if let Some(run_on_start) = run_on_start {
                script.run_on_start = run_on_start;
            }
            let updated = replace_scripts(client, &workspace_id, scripts).await?;
            print_scripts_result(&updated, json);
        }
        WorkspaceScriptAction::Remove { workspace_id, name } => {
            let mut scripts = client
                .api()
                .get_workspace(&workspace_id)
                .await?
                .scripts
                .unwrap_or_default();
            let before = scripts.len();
            scripts.retain(|s| s.name != name);
            if scripts.len() == before {
                return Err(not_declared(&workspace_id, &name));
            }
            let updated = replace_scripts(client, &workspace_id, scripts).await?;
            print_scripts_result(&updated, json);
        }
    }
    Ok(())
}

fn not_declared(workspace_id: &str, name: &str) -> ClientError {
    ClientError::Other(format!(
        "No script named \"{name}\". Run `moat workspaces scripts list {workspace_id}` to see what's declared."
    ))
}

/// A port in the 1-65535 range `scripts::replace`'s own editor enforces — kept
/// in sync here so a script declared through this read-modify-write path cannot
/// store a `0` the T3 UI's editor would refuse.
fn validate_port(port: i32) -> Result<u16, ClientError> {
    u16::try_from(port)
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| {
            ClientError::Other(format!(
                "`{port}` is not a port. Use 1-65535, or -1 to clear."
            ))
        })
}

/// Replace a workspace's whole Scripts list.
///
/// `UpdateWorkspaceRequest` patches only the fields set, so this touches
/// nothing else; a concurrent editor's write between the read and this one is
/// lost, same as any other `moat workspaces update`.
async fn replace_scripts(
    client: &MoatClient,
    workspace_id: &str,
    scripts: Vec<ScriptConfig>,
) -> Result<WorkspaceResponse, ClientError> {
    let req = UpdateWorkspaceRequest {
        scripts: Some(scripts),
        ..Default::default()
    };
    Ok(client.api().update_workspace(workspace_id, &req).await?)
}

fn print_scripts_result(updated: &WorkspaceResponse, json: bool) {
    let scripts = updated.scripts.clone().unwrap_or_default();
    output::emit(json, &scripts, || format_scripts_list(&scripts));
}

/// One line per script this workspace declares.
pub(crate) fn format_scripts_list(scripts: &[ScriptConfig]) {
    let rows: Vec<Vec<String>> = scripts
        .iter()
        .map(|s| {
            vec![
                s.name.clone(),
                s.label.clone().unwrap_or_default(),
                summarize_command(&s.command),
                s.port.map(|p| p.to_string()).unwrap_or_default(),
                if s.run_on_start { "yes" } else { "" }.to_string(),
            ]
        })
        .collect();
    output::print_table(&["NAME", "LABEL", "COMMAND", "PORT", "ON START"], &rows);
}

/// A multi-line command folded onto one line for a table (display only — the
/// stored command is untouched).
pub(crate) fn summarize_command(command: &str) -> String {
    let one_line = command.replace('\n', " \u{23ce} ");
    if one_line.chars().count() > 60 {
        format!("{}...", one_line.chars().take(57).collect::<String>())
    } else {
        one_line
    }
}

async fn run_credentials(
    client: &MoatClient,
    action: CredentialAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        CredentialAction::List { workspace_id } => {
            let creds = client
                .api()
                .list_workspace_credentials(&workspace_id)
                .await?;
            if json {
                output::print_json_typed(&creds);
            } else if creds.is_empty() {
                println!("No credentials pinned to workspace {workspace_id}.");
            } else {
                let rows: Vec<Vec<String>> = creds
                    .iter()
                    .map(|c| {
                        vec![
                            c.provider.to_string(),
                            c.env_key.clone(),
                            c.updated_by.clone(),
                            c.updated_at.clone(),
                        ]
                    })
                    .collect();
                output::print_table(&["PROVIDER", "ENV KEY", "UPDATED BY", "UPDATED"], &rows);
            }
        }
        CredentialAction::Set {
            workspace_id,
            provider,
            value,
            value_file,
        } => {
            let resolved = match value_file.as_deref() {
                Some(path) => read_value_file(path)?,
                None => read_value(value)?.ok_or_else(|| {
                    ClientError::Other("Either --value or --value-file is required".into())
                })?,
            };
            let stored = client
                .api()
                .set_workspace_credential(&workspace_id, &provider, &resolved)
                .await?;
            output::emit(json, &stored, || {
                println!(
                    "Pinned {} credential to workspace {} (delivered as {}).",
                    stored.provider, workspace_id, stored.env_key
                );
            });
        }
        CredentialAction::Remove {
            workspace_id,
            provider,
        } => {
            client
                .api()
                .delete_workspace_credential(&workspace_id, &provider)
                .await?;
            output::emit(
                json,
                &serde_json::json!({
                    "workspaceId": workspace_id,
                    "provider": provider,
                    "removed": true,
                }),
                || println!("Removed {provider} credential from workspace {workspace_id}."),
            );
        }
    }
    Ok(())
}

/// Read a credential value from `--value`, honouring `-` as "read stdin".
fn read_value(value: Option<String>) -> Result<Option<String>, ClientError> {
    use std::io::{IsTerminal, Read};
    match value.as_deref() {
        Some("-") => {
            if std::io::stdin().is_terminal() {
                return Err(ClientError::Other(
                    "Cannot read from stdin in interactive mode. Pipe input or use --value directly."
                        .into(),
                ));
            }
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| ClientError::Other(format!("Failed to read from stdin: {e}")))?;
            Ok(Some(buf.trim_end_matches('\n').to_string()))
        }
        Some(v) => Ok(Some(v.to_string())),
        None => Ok(None),
    }
}

fn read_value_file(path: &str) -> Result<String, ClientError> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| ClientError::Other(format!("Failed to read value file '{path}': {e}")))?;
    Ok(contents.trim_end_matches('\n').to_string())
}

fn parse_scope(s: &str) -> Option<Scope> {
    match s {
        "global" => Some(Scope::Global),
        "user" => Some(Scope::User),
        _ => None,
    }
}

fn parse_scope_arg(s: Option<&str>) -> Result<Option<Scope>, ClientError> {
    match s {
        None => Ok(None),
        Some(v) => parse_scope(v)
            .map(Some)
            .ok_or_else(|| ClientError::Other(format!("Invalid --scope '{v}' (global/user)"))),
    }
}

fn parse_json_opt<T: serde::de::DeserializeOwned>(
    raw: &Option<String>,
    flag: &str,
) -> Result<Option<T>, ClientError> {
    match raw.as_deref() {
        None => Ok(None),
        Some(s) => serde_json::from_str(s)
            .map(Some)
            .map_err(|e| ClientError::Other(format!("Invalid {flag}: {e}"))),
    }
}

fn format_workspace_list(workspaces: &[WorkspaceResponse]) {
    let rows: Vec<Vec<String>> = workspaces
        .iter()
        .map(|w| {
            let source = if w.synced_from_git {
                "git".to_string()
            } else if w.source_repository_id.is_some() {
                "manual (was git)".to_string()
            } else {
                "manual".to_string()
            };
            vec![w.id.clone(), w.name.clone(), w.kind.clone(), source]
        })
        .collect();
    output::print_table(&["ID", "NAME", "KIND", "SOURCE"], &rows);
}

fn format_workspace_detail(w: &WorkspaceResponse) {
    println!("ID:          {}", w.id);
    println!("Name:        {}", w.name);
    if let Some(ref d) = w.description {
        println!("Description: {d}");
    }
    println!("Kind:        {}", w.kind);
    println!("Scope:       {}", w.scope);
    if let Some(ref owner) = w.owner_user_id {
        println!("Owner:       {owner}");
    }

    let source_label = if w.synced_from_git {
        "git".to_string()
    } else if w.source_repository_id.is_some() {
        "manual (was git — overridden)".to_string()
    } else {
        "manual".to_string()
    };
    println!("Source:      {source_label}");
    if let Some(ref path) = w.source_config_path {
        println!("Config path: {path}");
    }
    if let Some(ref repo_id) = w.source_repository_id {
        println!("Source repo: {repo_id}");
    }
    if let Some(ref key) = w.source_key {
        println!("Source key:  {key}");
    }

    if let Some(ref img) = w.docker_image {
        println!("Docker image: {img}");
    }
    if let Some(docker_access) = w.docker_access {
        println!("Docker access: {docker_access}");
    }
    if let Some(ref svc) = w.sandbox_service_account {
        println!("Service account: {svc}");
    }
    if let Some(ref profile) = w.network_profile {
        println!("Network profile: {profile}");
    }
    if let Some(labels) = w.node_selector.as_ref().filter(|s| !s.is_empty()) {
        let rendered: Vec<String> = labels.iter().map(|(k, v)| format!("{k}={v}")).collect();
        println!("Node selector: {}", rendered.join(","));
    }
    if let Some(ref prompt) = w.system_prompt {
        let preview: String = prompt.chars().take(80).collect();
        let ellipsis = if prompt.len() > 80 { "…" } else { "" };
        println!("System prompt: {preview}{ellipsis}");
    }
    if !w.repos.is_empty() {
        println!("Repos ({}):", w.repos.len());
        for r in &w.repos {
            let primary = if r.is_primary { " [primary]" } else { "" };
            let branch = r.branch.as_deref().unwrap_or("default");
            println!("  {} branch={branch}{primary}", r.repository_id);
        }
    }
    if let Some(ref cmds) = w.setup_commands {
        if !cmds.is_empty() {
            println!("Setup commands:");
            for cmd in cmds {
                println!("  - {cmd}");
            }
        }
    }
}
