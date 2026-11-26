//! A Workspace as the project T3 groups conversations under.

use serde_json::{Value, json};

use super::model::default_model_selection;
use super::scalar::non_empty;
use super::scalar::timestamp;
use crate::nominal_path;
use moatless_workspace_types::schemas::workspace::ScriptConfig;
use moatless_workspaces::workspace::models::{
    ListedWorkspace, Workspace, WorkspaceIcon, WorkspaceSource,
};

/// The icons the contract accepts, and the one anything else becomes.
///
/// The contract's icon is a closed set that fails the whole listing to decode
/// on an unknown value, so an unrecognized icon is reported as the default
/// instead.
pub(super) const SCRIPT_ICONS: [&str; 6] = ["play", "test", "lint", "configure", "build", "debug"];

const DEFAULT_SCRIPT_ICON: &str = "play";

/// Whether the contract would accept this icon.
///
/// The read path tolerates anything and reports the default; the Task-script
/// decoder stores only icons this accepts.
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
pub(crate) fn project(listed: &ListedWorkspace) -> Value {
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
        "scriptsEditable": listed.readable && !is_git_synced(workspace) && !workspace.deleted,
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
/// [`WorkspaceIcon`] holds any string, but the client decodes it strictly: a
/// rejected shape takes the project off the client, so it's checked here
/// against the same bounds.
fn project_icon(icon: Option<&WorkspaceIcon>) -> Value {
    match icon {
        Some(WorkspaceIcon::Lucide { name, color }) => {
            let (name, color) = (name.trim(), color.trim());
            if is_lucide_icon_name(name) && PROJECT_ICON_COLORS.contains(&color) {
                json!({ "kind": "lucide", "name": name, "color": color })
            } else {
                Value::Null
            }
        }
        Some(WorkspaceIcon::Emoji { emoji }) => {
            let emoji = emoji.trim();
            // `isMaxLength` counts what JS counts, so the bound is UTF-16 units.
            if emoji.is_empty() || emoji.encode_utf16().count() > 32 {
                Value::Null
            } else {
                json!({ "kind": "emoji", "emoji": emoji })
            }
        }
        None => Value::Null,
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
    workspace.source == WorkspaceSource::Git
}

/// The workspace's declared Scripts.
///
/// Not a *server*: the Sandbox already runs it as a container, so a runnable
/// entry for it would start a second process on a bound port. Setup commands
/// are provisioning, not scripts either.
pub(super) fn scripts(workspace: &Workspace) -> Vec<Value> {
    workspace
        .scripts
        .iter()
        .flatten()
        .filter_map(|script| ui_script(script, None))
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
