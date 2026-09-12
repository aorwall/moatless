//! A Workspace as the project T3 groups conversations under.

use serde_json::{Value, json};
use tracing::warn;

use super::model::default_model_selection;
use super::scalar::non_empty;
use super::scalar::timestamp;
use crate::nominal_path;
use crate::workspace::{UiRpcListedWorkspace, WorkspaceView as Workspace};

/// The fields a script is read out of, narrowed from the workspace's stored
/// script config. Every field the canonical `ScriptConfig` defaults is optional
/// here too — one required field that is merely usually present costs the whole
/// workspace its scripts.
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

/// The icons the contract accepts, and the one anything else becomes.
///
/// The contract's icon is a closed set and an unknown value fails the whole
/// listing to decode. A workspace is free to store any string, so the value is
/// checked here rather than trusted: an unrecognized icon is reported as the
/// default instead of refusing the script, because an icon is not worth losing
/// a script over.
pub(super) const SCRIPT_ICONS: [&str; 6] = ["play", "test", "lint", "configure", "build", "debug"];

const DEFAULT_SCRIPT_ICON: &str = "play";

/// Whether the contract would accept this icon.
///
/// The read path tolerates anything and reports the default; the write path
/// ([`scripts::replace`](crate::scripts::replace)) has to decide whether to
/// *store* what it was handed, and stores nothing it could not read back.
#[must_use]
pub(crate) fn is_script_icon(icon: &str) -> bool {
    SCRIPT_ICONS.contains(&icon)
}

/// One project, in the shape the shell listing carries.
///
/// This is the contract's *shell* project, which is not quite its full project:
/// the shell one has no deletion timestamp, and soft-deleted workspaces never
/// reach here anyway.
///
/// # Two halves of a union, projected differently
///
/// The listing's projects are a union: the Workspaces this viewer may read,
/// plus the Workspaces a listed conversation references whether or not they may
/// read them. The widening is load-bearing — a conversation whose project is
/// absent is dropped by the client in silence — and what justifies it is the
/// *names*.
///
/// A start command is not a name: it carries paths, flags and whatever else is
/// in a server config, so the widened half is projected down to what the
/// contract needs to render a group and no more.
/// [`UiRpcListedWorkspace`](crate::workspace::UiRpcListedWorkspace) carries the
/// difference rather than having it re-derived.
///
/// That minimum still discloses the id, the name, the root synthesized *from*
/// the id, and the two timestamps. The timestamps stay because the contract
/// requires them and supplies no empty value for either.
pub(super) fn project(listed: &UiRpcListedWorkspace) -> Value {
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
        // Whether this project's scripts can be edited from the client at all.
        //
        // Reported rather than left for the write to refuse, because the answer
        // is a property of the Workspace and does not change with what somebody
        // types: a git-synced run config is reconciled from
        // `.moatless/workspaces.json`, so an edit here would be silently undone
        // by the next sync. The client hides the controls instead of offering
        // an action that can only fail. The widened half of the union is not
        // editable either — it is projected without its scripts — and neither
        // is a deleted one, which is listed only because open work still names
        // it.
        "scriptsEditable": listed.readable
            && !is_git_synced(workspace)
            && workspace.deleted != Some(true),
        "createdAt": timestamp(workspace.created_at),
        "updatedAt": timestamp(workspace.updated_at)
    })
}

/// The colours a `ProjectIconOverride` may name, from `ProjectIconColor` in the
/// client's `packages/contracts/src/orchestration.ts`, which is the authority.
///
/// A colour added there and not copied here draws the project's automatic glyph
/// until it is, which is the failure worth having: see [`project_icon`].
const PROJECT_ICON_COLORS: &[&str] = &[
    "gray", "red", "orange", "amber", "yellow", "lime", "green", "emerald", "teal", "cyan", "sky",
    "blue", "indigo", "violet", "purple", "fuchsia", "pink", "rose",
];

/// The Workspace's icon as a `ProjectIconOverride`, or `Null` when the stored
/// value is not one.
///
/// `workspaces.icon` is open jsonb any `PATCH` may write, and the client decodes
/// a project strictly — an icon its schema rejects fails the whole
/// `OrchestrationProject`, which takes the project and its conversations off the
/// client. So the shape is checked here rather than trusted, against the same
/// bounds the client's schema states.
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
/// The same string the REST surface's `ensure_workspace_mutable` tests, and the
/// same rule: a git-synced Workspace is read-only everywhere, because the file
/// it is reconciled from is the thing to edit.
fn is_git_synced(workspace: &Workspace) -> bool {
    workspace.source == "git"
}

/// The workspace's declared Scripts.
///
/// Only Scripts. The workspace's *servers* were mapped here once, which put a
/// runnable-looking entry in the client's top bar for something the Sandbox
/// already runs as a container — pressing it would have started a second
/// process on a bound port. Setup commands are provisioning and are likewise
/// not scripts.
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
        .iter()
        .filter_map(|script| {
            // Two of a script's fields are non-empty strings, and one script
            // the client cannot read takes the entire listing down with it. A
            // script that cannot fill them is left out rather than emitted
            // malformed.
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
            Some(json!({
                // The script's name: stable across reads, and unique within the
                // workspace. It is the identity rather than a generated id
                // because it has to survive a git reconcile.
                "id": id,
                "name": name,
                "command": command,
                "icon": icon,
                // The client's name for `run_on_start`. Moatless has no
                // worktrees — a Sandbox is the isolation — so the backend runs
                // the flagged Script itself when the Sandbox reports Ready
                // (`startup_scripts`), and the client only labels it "(setup)".
                "runOnWorktreeCreate": script.run_on_start,
                // A script that serves a port is worth auto-opening a preview
                // for; one that prints to a console is not.
                "autoOpenPreview": script.port.is_some(),
                // The port itself, so the client can show and edit it. This is
                // the fork's stand-in for upstream's `previewUrl`, which a
                // Moatless script cannot carry: the published address only
                // exists once a Task has a sandbox serving that port, and
                // `scripts.run` returns the real one. `previewUrl` is therefore
                // omitted rather than guessed.
                "port": script.port
            }))
        })
        .collect()
}
