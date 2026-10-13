//! A Workspace as the project T3 groups conversations under.

use serde_json::{Value, json};
use tracing::warn;

use super::model::default_model_selection;
use super::scalar::non_empty;
use super::scalar::timestamp;
use crate::nominal_path;
use crate::workspace::{UiRpcListedWorkspace, WorkspaceView as Workspace};
use moatless_workspace_types::schemas::repository::ScriptConfig;

/// The fields a script is read out of, narrowed from the workspace's stored
/// config.
///
/// Every field the canonical `ScriptConfig` defaults is optional here too,
/// since one required field merely usually present costs the whole workspace
/// its scripts.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UiRpcScriptConfig {
    name: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    command: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    run_on_start: bool,
}

impl From<UiRpcScriptConfig> for ScriptConfig {
    fn from(script: UiRpcScriptConfig) -> Self {
        ScriptConfig {
            name: script.name,
            label: script.label,
            command: script.command,
            icon: script.icon,
            port: script.port,
            // No wire field decodes into it, and [`ui_script`] shapes nothing
            // out of it: a Script's environment is the Sandbox's business.
            env_vars: None,
            run_on_start: script.run_on_start,
        }
    }
}

/// The icons the contract accepts, and the one anything else becomes.
///
/// The contract's icon is a closed set that fails the whole listing to decode
/// on an unknown value, so an unrecognized icon is reported as the default
/// instead.
pub(super) const SCRIPT_ICONS: [&str; 6] = ["play", "test", "lint", "configure", "build", "debug"];

const DEFAULT_SCRIPT_ICON: &str = "play";

/// Whether the contract would accept this icon.
///
/// The read path tolerates anything and reports the default; the write path
/// ([`scripts::replace`](crate::scripts::replace)) stores nothing it could not
/// read back.
#[must_use]
pub(crate) fn is_script_icon(icon: &str) -> bool {
    SCRIPT_ICONS.contains(&icon)
}

/// One project, in the shape the shell listing carries; the contract's *shell*
/// project, with no deletion timestamp.
///
/// The listing unions readable Workspaces with ones only referenced by a
/// conversation; the widening is load-bearing, since a conversation whose
/// project is absent is dropped by the client in silence.
pub(crate) fn project(listed: &UiRpcListedWorkspace) -> Value {
    let workspace = &listed.workspace;
    json!({
        "id": workspace.id,
        // `workspaces.name` is NOT NULL but not non-empty, and the contract
        // refuses a blank title. An unnamed workspace would take the whole
        // listing down with it, so it falls back to its id.
        "title": non_empty(&workspace.name).unwrap_or_else(|| workspace.id.clone()),
        "workspaceRoot": nominal_path::nominal_root(&workspace.id),
        // The environment advertises no repository identity, so the client
        // never groups projects across environments by git remote.
        "repositoryIdentity": Value::Null,
        // The Workspace's own defaults, which the client uses to seed a new
        // thread. Only the widened half is withheld: neither is a name, but
        // both are configuration the viewer may not read.
        "projectIcon": if listed.readable {
            project_icon(workspace.icon.as_ref())
        } else {
            Value::Null
        },
        "defaultModelSelection": if listed.readable {
            default_model_selection(workspace)
        } else {
            Value::Null
        },
        // Required by the contract, so the narrowed shape is an empty list
        // rather than an absent field.
        "scripts": if listed.readable { scripts(workspace) } else { Vec::new() },
        // Reported, not left for the write to refuse: a git-synced run config
        // is reconciled from `.moatless/workspaces.json`, so an edit here
        // would be undone by the next sync — the client hides the controls
        // instead. The widened half of the union isn't editable either (no
        // scripts projected), nor is a deleted one.
        "scriptsEditable": listed.readable
            && !is_git_synced(workspace)
            && workspace.deleted != Some(true),
        "createdAt": timestamp(workspace.created_at),
        "updatedAt": timestamp(workspace.updated_at)
    })
}

/// The colours a `ProjectIconOverride` may name, from `ProjectIconColor` in the
/// client's `packages/contracts/src/orchestration.ts`, the authority.
///
/// A colour added there and not copied here draws the project's automatic glyph
/// until it is — the failure worth having.
const PROJECT_ICON_COLORS: &[&str] = &[
    "gray", "red", "orange", "amber", "yellow", "lime", "green", "emerald", "teal", "cyan", "sky",
    "blue", "indigo", "violet", "purple", "fuchsia", "pink", "rose",
];

/// The Workspace's icon as a `ProjectIconOverride`, or `Null` when the stored
/// value is not one.
///
/// `workspaces.icon` is open jsonb any `PATCH` may write, but the client
/// decodes it strictly: a rejected shape takes the project off the client, so
/// it's checked here against the same bounds.
fn project_icon(icon: Option<&Value>) -> Value {
    let icon = icon.unwrap_or(&Value::Null);
    let field = |name: &str| {
        icon.get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
    };
    match icon.get("kind").and_then(Value::as_str) {
        Some("lucide") => {
            let (name, color) = (field("name"), field("color"));
            if is_lucide_icon_name(name) && PROJECT_ICON_COLORS.contains(&color) {
                json!({ "kind": "lucide", "name": name, "color": color })
            } else {
                Value::Null
            }
        }
        Some("emoji") => {
            let emoji = field("emoji");
            // `isMaxLength` counts what JS counts, so the bound is UTF-16 units.
            if emoji.is_empty() || emoji.encode_utf16().count() > 32 {
                Value::Null
            } else {
                json!({ "kind": "emoji", "emoji": emoji })
            }
        }
        _ => Value::Null,
    }
}

/// The client's `ProjectLucideIconName`: `^[a-z0-9]+(?:-[a-z0-9]+)*$`, at most
/// 64 characters.
fn is_lucide_icon_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Whether git owns this Workspace's run config.
///
/// The same rule the REST surface's `ensure_workspace_mutable` tests: a
/// git-synced Workspace is read-only everywhere, since the file it reconciles
/// from is the thing to edit.
fn is_git_synced(workspace: &Workspace) -> bool {
    workspace.source == "git"
}

/// The workspace's declared Scripts.
///
/// Not a *server*: the Sandbox already runs it as a container, so a runnable
/// entry for it would start a second process on a bound port. Setup commands
/// are provisioning, not scripts either.
pub(super) fn scripts(workspace: &Workspace) -> Vec<Value> {
    let Some(scripts) = workspace.scripts.as_ref() else {
        return Vec::new();
    };
    let scripts: Vec<UiRpcScriptConfig> = match serde_json::from_value(scripts.clone()) {
        Ok(scripts) => scripts,
        Err(error) => {
            warn!(
                %error,
                workspace_id = %workspace.id,
                "Workspace scripts config is unreadable; reporting no scripts"
            );
            return Vec::new();
        }
    };

    scripts
        .into_iter()
        .filter_map(|script| ui_script(&script.into(), None))
        .collect()
}

/// One Script in the shape the contract's `ProjectScript` takes, or `None`
/// where a field the client requires is empty.
///
/// `scope` names the side that declared this Script, for the listings that
/// merge two of them (`scripts.listForThread`). The project listing passes
/// `None` and the field is left out: every Script there is its project's.
pub(crate) fn ui_script(script: &ScriptConfig, scope: Option<&str>) -> Option<Value> {
    // Two of a script's fields are non-empty strings, and one script the client
    // cannot read takes the entire listing down with it. A script that cannot
    // fill them is left out rather than emitted malformed.
    let id = non_empty(&script.name)?;
    let command = non_empty(&script.command)?;
    let name = script
        .label
        .as_deref()
        .and_then(non_empty)
        .unwrap_or_else(|| id.clone());
    let icon = script
        .icon
        .as_deref()
        .filter(|icon| SCRIPT_ICONS.contains(icon))
        .unwrap_or(DEFAULT_SCRIPT_ICON);
    let mut shaped = json!({
        // The script's name: stable across reads, and unique within the
        // workspace. It is the identity rather than a generated id because it
        // has to survive a git reconcile.
        "id": id,
        "name": name,
        "command": command,
        "icon": icon,
        // The client's name for `run_on_start`. Moatless has no worktrees — a
        // Sandbox is the isolation — so the backend runs the flagged Script
        // itself when the Sandbox reports Ready (`startup_scripts`), and the
        // client only labels it "(setup)".
        "runOnWorktreeCreate": script.run_on_start,
        // A script that serves a port is worth auto-opening a preview for; one
        // that prints to a console is not.
        "autoOpenPreview": script.port.is_some(),
        // The stand-in for upstream's `previewUrl`, which a Moatless script
        // cannot carry: the published address exists only once a Task has a
        // sandbox serving the port, and `scripts.run` returns it. `previewUrl`
        // is omitted rather than guessed.
        "port": script.port
    });
    if let Some(scope) = scope {
        shaped["scope"] = Value::from(scope);
    }
    Some(shaped)
}
