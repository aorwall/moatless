//! Domain persistence stays inside the crate that owns its tables.
//!
//! The check reads the module tree rather than the filesystem in both the
//! default and `test-support` builds, so no alias, glob, re-export, signature,
//! or feature gate can carry a store out of its crate.
//!
//! [`CONCERNS`] is the migration inventory the rule reconciles against the
//! workspace and against [`TABLES`]; an added, moved, or deleted root or table
//! fails here until the inventory records it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::Result;

use crate::Violation;
use crate::public_api::{Export, Scope, exports_in};
use crate::table_ownership::TABLES;
use crate::workspace::Workspace;

pub const RULE: &str = "persistence-export";

/// One persistence concern: where its stores sit, which tables it owns, and the
/// migration slice that makes it private.
pub struct Concern {
    /// The concern's name in the persistence migration.
    pub name: &'static str,
    /// The crate that declares the stores.
    pub owner: &'static str,
    /// Module paths without the `.rs`, each holding persistence. A path covers
    /// `<path>.rs` and everything under `<path>/`.
    pub roots: &'static [&'static str],
    /// The tables from [`TABLES`] this concern owns. Every table is owned by
    /// exactly one concern, which is not the same as being read by one: a
    /// reader of somebody else's table is a `table-ownership` question.
    pub tables: &'static [&'static str],
    /// The persistence-migration slice that privatizes it.
    pub slice: u8,
}

/// Every persistence root in the workspace, at `fbac88ec6362`.
///
/// A root is a module the DAO layout claims (`dao.rs`, `*_dao.rs`, `dao/`), or
/// a store outside that layout listed here so reconciliation still finds it.
pub const CONCERNS: &[Concern] = &[
    Concern {
        name: "file metadata",
        owner: "moatless-core",
        roots: &["crates/core/src/file_storage/dao"],
        tables: &["file_metadata"],
        slice: 1,
    },
    Concern {
        name: "conversation feed",
        owner: "moatless-core",
        roots: &["crates/core/src/change_feed/conversation/dao"],
        tables: &["task_conversation_changes", "task_conversation_position"],
        slice: 1,
    },
    Concern {
        name: "events feed",
        owner: "moatless-core",
        roots: &["crates/core/src/change_feed/events/dao"],
        tables: &[
            "task_event_changes",
            "task_event_dispatch_queue",
            "task_event_position",
        ],
        slice: 1,
    },
    Concern {
        name: "listing feed",
        owner: "moatless-core",
        roots: &["crates/core/src/change_feed/listing/dao"],
        tables: &[
            "task_listing_changes",
            "task_listing_position",
            "workspace_listing_changes",
        ],
        slice: 1,
    },
    Concern {
        name: "preview feed",
        owner: "moatless-core",
        roots: &["crates/core/src/change_feed/preview/dao"],
        tables: &["preview_tab_changes", "preview_tab_position"],
        slice: 1,
    },
    Concern {
        name: "pull request host changes",
        owner: "moatless-core",
        roots: &["crates/core/src/change_feed/pull_requests/dao"],
        tables: &["pull_request_host_changes"],
        slice: 1,
    },
    Concern {
        name: "terminal feed",
        owner: "moatless-core",
        roots: &["crates/core/src/change_feed/terminal/dao"],
        tables: &["terminal_session_changes", "terminal_session_position"],
        slice: 1,
    },
    Concern {
        name: "adapter connections",
        owner: "moatless-adapter-model",
        roots: &["crates/adapter-model/src/connections/dao"],
        tables: &["adapter_connections", "adapter_connection_events"],
        slice: 2,
    },
    Concern {
        name: "users",
        owner: "moatless-identity",
        roots: &["crates/identity/src/auth/users_dao"],
        tables: &["user_provider_ids", "users"],
        slice: 3,
    },
    Concern {
        name: "teams",
        owner: "moatless-identity",
        roots: &["crates/identity/src/team/dao"],
        tables: &["team_members", "teams"],
        slice: 3,
    },
    Concern {
        name: "account creation",
        owner: "moatless-identity",
        roots: &["crates/identity/src/auth/account_dao"],
        tables: &[],
        slice: 4,
    },
    Concern {
        name: "api keys",
        owner: "moatless-identity",
        roots: &["crates/identity/src/auth/api_keys_dao"],
        tables: &["api_keys"],
        slice: 4,
    },
    Concern {
        name: "passwords",
        owner: "moatless-identity",
        roots: &[
            "crates/identity/src/auth/password/dao",
            "crates/identity/src/auth/password/throttle_dao",
        ],
        tables: &["auth_rate_limits", "user_credentials"],
        slice: 4,
    },
    Concern {
        name: "oidc",
        owner: "moatless-identity",
        roots: &["crates/identity/src/auth/oidc_dao"],
        tables: &["auth_transactions", "oidc_identities"],
        slice: 4,
    },
    Concern {
        name: "device pairing",
        owner: "moatless-identity",
        roots: &["crates/identity/src/auth/pairing_dao"],
        tables: &["device_pairing_codes"],
        slice: 4,
    },
    Concern {
        name: "sessions",
        owner: "moatless-identity",
        roots: &["crates/identity/src/auth/sessions/dao"],
        tables: &["auth_audit_events", "auth_sessions"],
        slice: 4,
    },
    Concern {
        name: "provider tokens",
        owner: "moatless-identity",
        roots: &[
            "crates/identity/src/auth/provider_token_auth_dao",
            "crates/identity/src/auth/provider_token_dao",
        ],
        tables: &["provider_tokens"],
        slice: 5,
    },
    Concern {
        name: "secrets",
        owner: "moatless-identity",
        roots: &["crates/identity/src/secret/dao"],
        tables: &["secrets"],
        slice: 6,
    },
    Concern {
        name: "secret audit",
        owner: "moatless-identity",
        roots: &["crates/identity/src/secret/audit_dao"],
        tables: &["secret_audit_events"],
        slice: 6,
    },
    Concern {
        name: "adapter apps",
        owner: "moatless-identity",
        roots: &[
            "crates/identity/src/adapter_apps/app_dao",
            "crates/identity/src/adapter_apps/dao",
        ],
        tables: &[
            "adapter_app_audit_events",
            "adapter_app_migrations",
            "adapter_app_secrets",
            "adapter_apps",
        ],
        slice: 6,
    },
    Concern {
        name: "forgejo instances",
        owner: "moatless-identity",
        roots: &["crates/identity/src/forgejo/dao"],
        tables: &["forgejo_instances"],
        slice: 6,
    },
    Concern {
        name: "repositories",
        owner: "moatless-workspaces",
        roots: &[
            "crates/workspaces/src/repository/dao",
            "crates/workspaces/src/repository/secret_migration_dao",
        ],
        tables: &["repositories"],
        slice: 7,
    },
    Concern {
        name: "workspaces",
        owner: "moatless-workspaces",
        roots: &["crates/workspaces/src/workspace/dao"],
        tables: &["workspace_repos", "workspaces"],
        slice: 7,
    },
    Concern {
        name: "workspace credentials",
        owner: "moatless-workspaces",
        roots: &["crates/workspaces/src/workspace/credentials/dao"],
        tables: &["workspace_credentials"],
        slice: 8,
    },
    Concern {
        name: "plugins",
        owner: "moatless-workspaces",
        roots: &["crates/workspaces/src/plugin/dao"],
        tables: &["plugin_activations", "plugin_skills", "plugins"],
        slice: 8,
    },
    Concern {
        name: "task graph",
        owner: "moatless-tasks",
        roots: &[
            "crates/tasks/src/task/access_dao",
            "crates/tasks/src/task/dao",
            "crates/tasks/src/task/pin_dao",
            "crates/tasks/src/task/sandbox_reach_dao",
            "crates/tasks/src/task/settle_dao",
            "crates/tasks/src/task/snooze_dao",
            "crates/tasks/src/task/switch_dao",
            "crates/tasks/src/task_listing/listed_dao",
            "crates/tasks/src/task_listing/names_dao",
            "crates/tasks/src/search/dao",
        ],
        tables: &["task_follows", "task_read_receipts", "tasks"],
        slice: 9,
    },
    Concern {
        name: "task relations",
        owner: "moatless-tasks",
        roots: &["crates/tasks/src/task/relations_dao"],
        tables: &["task_relations"],
        slice: 9,
    },
    Concern {
        name: "task repositories",
        owner: "moatless-tasks",
        roots: &["crates/tasks/src/task/repositories_dao"],
        tables: &["task_repositories"],
        slice: 9,
    },
    Concern {
        name: "task forks",
        owner: "moatless-tasks",
        roots: &[
            "crates/tasks/src/task/fork_dao",
            "crates/tasks/src/task/merge_back_dao",
            "crates/tasks/src/task/side_chat_dao",
        ],
        tables: &["task_forks", "task_merge_backs"],
        slice: 9,
    },
    Concern {
        name: "artifacts",
        owner: "moatless-tasks",
        roots: &["crates/tasks/src/artifact/dao"],
        tables: &["task_artifacts", "task_artifact_versions"],
        slice: 9,
    },
    Concern {
        name: "record collections",
        owner: "moatless-tasks",
        roots: &["crates/tasks/src/collection/dao"],
        tables: &[
            "record_collections",
            "collection_records",
            "collection_record_changes",
        ],
        slice: 9,
    },
    Concern {
        name: "review guides",
        owner: "moatless-tasks",
        roots: &["crates/tasks/src/review_guide/dao"],
        tables: &[
            "pull_request_review_guide_runs",
            "pull_request_review_guides",
        ],
        slice: 9,
    },
    Concern {
        name: "messages",
        owner: "moatless-tasks",
        roots: &[
            "crates/tasks/src/conversation/dao",
            "crates/tasks/src/conversation/read_dao",
            "crates/tasks/src/conversation/replay_dao",
            "crates/tasks/src/conversation/transcript_dao",
            "crates/tasks/src/message/dao",
            "crates/tasks/src/message/external_events_dao",
            "crates/tasks/src/message/recovery_dao",
            "crates/tasks/src/task/rewind_dao",
        ],
        tables: &[
            "conversation_frozen",
            "conversation_messages",
            "conversation_usage",
        ],
        slice: 10,
    },
    Concern {
        name: "task activities",
        owner: "moatless-tasks",
        roots: &["crates/tasks/src/activities/dao"],
        tables: &["conversation_activities"],
        slice: 10,
    },
    Concern {
        name: "outbound events",
        owner: "moatless-tasks",
        roots: &["crates/tasks/src/task/outbound_events_dao"],
        tables: &["task_outbound_events"],
        slice: 10,
    },
    Concern {
        name: "task turns",
        owner: "moatless-tasks",
        roots: &[
            "crates/tasks/src/task_turn/dao",
            "crates/tasks/src/task_turn/usage_dao",
        ],
        tables: &["task_rewound_turns", "task_turns"],
        slice: 11,
    },
    Concern {
        name: "task events",
        owner: "moatless-tasks",
        roots: &["crates/tasks/src/task_event/dao"],
        tables: &["task_events"],
        slice: 11,
    },
    // Writes `task_events` and reads `conversation_activities`, both owned
    // above; it holds no table of its own.
    Concern {
        name: "secret requests",
        owner: "moatless-tasks",
        roots: &["crates/tasks/src/secret_request/dao"],
        tables: &[],
        slice: 11,
    },
    Concern {
        name: "inbox",
        owner: "moatless-tasks",
        roots: &["crates/tasks/src/inbox/dao"],
        tables: &["task_inbox"],
        slice: 11,
    },
    Concern {
        name: "trajectory",
        owner: "moatless-tasks",
        roots: &["crates/tasks/src/trajectory/dao"],
        tables: &["trajectory_events", "trajectory_sessions"],
        slice: 11,
    },
    Concern {
        name: "tags",
        owner: "moatless-tasks",
        roots: &["crates/tasks/src/tag/dao"],
        tables: &["tags", "task_tags"],
        slice: 12,
    },
    Concern {
        name: "feedback",
        owner: "moatless-tasks",
        roots: &["crates/tasks/src/feedback/dao"],
        tables: &["feedback"],
        slice: 12,
    },
    Concern {
        name: "memory",
        owner: "moatless-tasks",
        roots: &["crates/tasks/src/memory/dao"],
        tables: &[
            "memory_entries",
            "memory_events",
            "memory_reads",
            "memory_seen",
        ],
        slice: 12,
    },
    Concern {
        name: "loop aggregate",
        owner: "moatless-loops",
        roots: &["crates/loops/src/loops/dao"],
        tables: &["loop_configs", "loops", "schedules", "subscriptions"],
        slice: 13,
    },
    Concern {
        name: "task bindings",
        owner: "moatless-loops",
        roots: &["crates/loops/src/bindings/dao"],
        tables: &["task_bindings"],
        slice: 14,
    },
    Concern {
        name: "inbound message buffer",
        owner: "moatless-loops",
        roots: &["crates/loops/src/bindings/buffer/dao"],
        tables: &["inbound_message_buffer"],
        slice: 14,
    },
    Concern {
        name: "adapter delivery",
        owner: "moatless-loops",
        roots: &["crates/loops/src/adapters/delivery/dao"],
        tables: &["adapter_delivery"],
        slice: 15,
    },
    Concern {
        name: "firing receipts",
        owner: "moatless-loops",
        roots: &["crates/loops/src/loops/firing/receipts_dao"],
        tables: &["adapter_event_receipts", "adapter_task_creation_claims"],
        slice: 15,
    },
    Concern {
        name: "sandbox lifecycle",
        owner: "moatless-sandbox-control",
        roots: &[
            "crates/sandbox-control/src/lifecycle/dao",
            "crates/sandbox-control/src/lifecycle/db",
        ],
        tables: &["sandbox_unreported_commands", "sandboxes"],
        slice: 16,
    },
    Concern {
        name: "t3 conversation projection",
        owner: "moatless-t3code",
        roots: &["crates/t3code/src/dao"],
        tables: &[],
        slice: 17,
    },
];

/// Database infrastructure every band may name: the pool, the migrations and
/// the access predicates. Widening this list is a scope change, not a fix.
const INFRASTRUCTURE: &[(&str, &str)] = &[("moatless-core", "crates/core/src/db")];

/// The change-feed transaction interface, enumerated path by path.
///
/// Each entry has a caller outside `moatless-core`, so a new export from either
/// feed store fails here until this list records it.
const FEED_OPERATIONS: &[(&str, &str)] = &[
    (
        "moatless-core",
        "change_feed::conversation::allocate_position",
    ),
    ("moatless-core", "change_feed::conversation::changes_from"),
    ("moatless-core", "change_feed::conversation::current_head"),
    ("moatless-core", "change_feed::conversation::current_heads"),
    ("moatless-core", "change_feed::conversation::prune_changes"),
    (
        "moatless-core",
        "change_feed::conversation::record_activity_change",
    ),
    (
        "moatless-core",
        "change_feed::conversation::record_message_change",
    ),
    (
        "moatless-core",
        "change_feed::conversation::record_pull_request_change",
    ),
    (
        "moatless-core",
        "change_feed::conversation::record_revert_change",
    ),
    (
        "moatless-core",
        "change_feed::conversation::record_session_change",
    ),
    ("moatless-core", "change_feed::events::changes_after"),
    ("moatless-core", "change_feed::events::claim_dispatch"),
    ("moatless-core", "change_feed::events::current_position"),
    ("moatless-core", "change_feed::events::dispatch_depth"),
    ("moatless-core", "change_feed::events::prune_changes"),
    ("moatless-core", "change_feed::events::record_event"),
    ("moatless-core", "change_feed::events::release_dispatch"),
    ("moatless-core", "change_feed::listing::changes_after"),
    ("moatless-core", "change_feed::listing::current_head"),
    ("moatless-core", "change_feed::listing::current_position"),
    ("moatless-core", "change_feed::listing::prune_changes"),
    ("moatless-core", "change_feed::listing::record_change"),
    (
        "moatless-core",
        "change_feed::listing::record_workspace_change",
    ),
    ("moatless-core", "change_feed::preview::current_position"),
    ("moatless-core", "change_feed::preview::opens_after"),
    ("moatless-core", "change_feed::preview::prune_opens"),
    ("moatless-core", "change_feed::preview::record_open"),
    ("moatless-core", "change_feed::pull_requests::host_changes"),
    (
        "moatless-core",
        "change_feed::pull_requests::record_host_change",
    ),
    ("moatless-core", "change_feed::terminal::current_position"),
    ("moatless-core", "change_feed::terminal::prune_starts"),
    ("moatless-core", "change_feed::terminal::record_start"),
    ("moatless-core", "change_feed::terminal::starts_after"),
];

/// A live connection or transaction. `PgPool` is deliberately absent: the plan
/// lets an owner's composition function take the shared pool.
const CONNECTIONS: &[&str] = &[
    "PgConnection",
    "PgExecutor",
    "PgTransaction",
    "PoolConnection",
    "Transaction",
];

/// One crate's persistence layout, as the judgement sees it.
struct Layout<'a> {
    /// Module paths holding this crate's stores.
    roots: Vec<&'a str>,
    /// Module paths whose exports the plan allows.
    allowed: Vec<&'a str>,
    /// Written paths the plan allows out one by one: the temporary shared
    /// contracts, and the change-feed transaction interface.
    published: Vec<&'a str>,
    /// Persistence type names this crate declares and does not export.
    private: BTreeSet<String>,
}

impl Layout<'_> {
    /// Why an export carries persistence out of the crate, if it does.
    fn exposure(&self, export: &Export) -> Option<&'static str> {
        if covered(&self.allowed, &export.file) || self.published.contains(&&*export.written()) {
            return None;
        }
        if covered(&self.roots, &export.file) {
            return Some("a persistence module exports it");
        }
        if store_name(&export.name) {
            return Some("its name is a store's");
        }
        export
            .signature
            .iter()
            .any(|name| self.private.contains(name) || CONNECTIONS.contains(&name.as_str()))
            .then_some("its public signature names private persistence")
    }
}

/// Whether a module path covers a file. A path covers `<path>.rs` and
/// everything under `<path>/`, which is how Rust lays a module out.
fn covers(root: &str, file: &str) -> bool {
    file.strip_prefix(root)
        .is_some_and(|rest| rest == ".rs" || rest.starts_with('/'))
}

fn covered(roots: &[&str], file: &str) -> bool {
    roots.iter().any(|root| covers(root, file))
}

/// Whether the DAO layout claims this file.
fn dao_layout(file: &str) -> bool {
    file.ends_with("/dao.rs") || file.ends_with("_dao.rs") || file.contains("/dao/")
}

/// Whether a name is a store's, whatever module it sits in.
///
/// Matches the `Dao` suffix or the `Pg` prefix; a domain value is never
/// classified from its suffix alone.
fn store_name(name: &str) -> bool {
    name.ends_with("Dao") || name.strip_prefix("Pg").is_some_and(|rest| !rest.is_empty())
}

/// Every type a persistence root declares, exported or not.
fn declared(roots: &[&str], sources: &BTreeMap<String, String>) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    for text in sources
        .iter()
        .filter(|(file, _)| covered(roots, file))
        .map(|(_, text)| text)
    {
        for item in syn::parse_file(text)?.items {
            match item {
                syn::Item::Struct(s) => out.insert(s.ident.to_string()),
                syn::Item::Enum(e) => out.insert(e.ident.to_string()),
                syn::Item::Trait(t) => out.insert(t.ident.to_string()),
                syn::Item::Type(t) => out.insert(t.ident.to_string()),
                _ => continue,
            };
        }
    }
    Ok(out)
}

/// Every export of one library, in both build scopes, under each path an
/// importer can write.
fn surface(root: &str, sources: &BTreeMap<String, String>) -> Result<Vec<Export>> {
    let mut out: BTreeMap<String, Export> = BTreeMap::new();
    for scope in [Scope::Default, Scope::TestSupport] {
        for export in exports_in(sources, root, scope)? {
            out.entry(export.written()).or_insert(export);
        }
    }
    Ok(out.into_values().collect())
}

fn paths<'a>(entries: impl Iterator<Item = (&'a str, &'a str)>, package: &str) -> Vec<&'a str> {
    entries
        .filter(|(owner, _)| *owner == package)
        .map(|(_, path)| path)
        .collect()
}

/// What one crate keeps private, what it may publish, and where its persistence
/// sits.
fn layout_of<'a>(
    package: &str,
    sources: &BTreeMap<String, String>,
    surface: &[Export],
) -> Result<Layout<'a>> {
    let roots = paths(CONCERNS.iter().flat_map(concern_roots), package);
    let exported: BTreeSet<&str> = surface.iter().map(|e| e.name.as_str()).collect();
    let published = paths(FEED_OPERATIONS.iter().copied(), package);
    Ok(Layout {
        private: declared(&roots, sources)?
            .into_iter()
            .filter(|name| !exported.contains(name.as_str()))
            .collect(),
        allowed: paths(INFRASTRUCTURE.iter().copied(), package),
        published,
        roots,
    })
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    let mut found: BTreeSet<(String, String)> = BTreeSet::new();

    for member in ws.members() {
        let Some(target) = member.targets.iter().find(|t| t.library()) else {
            continue;
        };
        let root = ws.relative(&target.path);
        let sources: BTreeMap<String, String> = ws.sources(member)?.into_iter().collect();

        let surface = surface(&root, &sources)?;
        let layout = layout_of(&member.name, &sources, &surface)?;

        found.extend(
            sources
                .keys()
                .filter(|file| dao_layout(file))
                .map(|file| (member.name.clone(), file.clone())),
        );

        for export in &surface {
            let Some(why) = layout.exposure(export) else {
                continue;
            };
            out.push(Violation::at(
                RULE,
                format!("{} exports {}", member.name, export.written()),
                format!("`{}` is domain persistence and {why}", export.name),
                export.file.clone(),
            ));
        }
    }

    out.extend(root_drift(ws, &found));
    out.extend(table_drift());
    out.sort();
    Ok(out)
}

/// Where the persistence roots [`CONCERNS`] records and the ones on disk
/// disagree.
fn root_drift(ws: &Workspace, found: &BTreeSet<(String, String)>) -> Vec<Violation> {
    let mut out = Vec::new();
    for (package, file) in found {
        let roots = paths(CONCERNS.iter().flat_map(concern_roots), package);
        if !covered(&roots, file) {
            out.push(Violation::at(
                RULE,
                format!("inventory: {file}"),
                "record this persistence root in the migration inventory",
                file.clone(),
            ));
        }
    }
    for (concern, root) in CONCERNS
        .iter()
        .flat_map(|c| c.roots.iter().map(move |r| (c, r)))
    {
        if !ws.root.join(format!("{root}.rs")).exists() && !ws.root.join(root).is_dir() {
            out.push(Violation::at(
                RULE,
                format!("inventory: {root} is gone"),
                format!(
                    "drop the {} entry for a persistence root that no longer exists",
                    concern.name
                ),
                (*root).to_string(),
            ));
        }
    }
    out
}

fn concern_roots(c: &'static Concern) -> impl Iterator<Item = (&'static str, &'static str)> {
    c.roots.iter().map(|root| (c.owner, *root))
}

/// Where the tables [`CONCERNS`] claims and [`TABLES`] disagree.
fn table_drift() -> Vec<Violation> {
    let mut claimed: BTreeMap<&str, Vec<&Concern>> = BTreeMap::new();
    for concern in CONCERNS {
        for table in concern.tables {
            claimed.entry(table).or_default().push(concern);
        }
    }
    let mut out = Vec::new();
    for (table, owner) in TABLES {
        let message = match claimed.get(table).map(Vec::as_slice) {
            Some([concern]) if concern.owner == *owner => continue,
            Some([concern]) => format!(
                "{table} is owned by {owner}, and the inventory gives it to {}",
                concern.owner
            ),
            Some(_) => format!("{table} is claimed by more than one persistence concern"),
            None => format!("give {table} to the persistence concern that owns it"),
        };
        out.push(Violation::at(
            RULE,
            format!("inventory: table {table}"),
            message,
            "tooling/arch-rules/src/persistence_export.rs".to_string(),
        ));
    }
    out
}

/// Render the persistence inventory as Markdown, for `arch-report --persistence-inventory`.
///
/// The caller matrix each slice needs lives here rather than in [`CONCERNS`],
/// as a snapshot the rule asserts nothing about.
pub fn inventory(ws: &Workspace) -> Result<String> {
    let mut concerns = Vec::new();
    let mut unrooted = Vec::new();
    let imports = resolved_imports(ws)?;
    for member in ws.members() {
        let Some(target) = member.targets.iter().find(|t| t.library()) else {
            continue;
        };
        let root = ws.relative(&target.path);
        let sources: BTreeMap<String, String> = ws.sources(member)?.into_iter().collect();
        let surface = surface(&root, &sources)?;
        let layout = layout_of(&member.name, &sources, &surface)?;
        unrooted.extend(
            surface
                .iter()
                .filter(|e| layout.exposure(e).is_some() && !covered(&layout.roots, &e.file))
                .map(|e| (member.name.clone(), e.written(), e.file.clone())),
        );
        for concern in CONCERNS.iter().filter(|c| c.owner == member.name) {
            let files: Vec<String> = sources
                .keys()
                .filter(|file| covered(concern.roots, file))
                .cloned()
                .collect();
            let modules: Vec<Vec<String>> = files
                .iter()
                .filter_map(|file| module_path(&root, file))
                .collect();
            let exports: Vec<&Export> = surface
                .iter()
                .filter(|e| covered(concern.roots, &e.file))
                .collect();
            let lib = member.name.replace('-', "_");
            let mut reach: Vec<String> = modules
                .iter()
                .map(|path| format!("{lib}::{}", path.join("::")))
                .collect();
            reach.extend(exports.iter().map(|e| format!("{lib}::{}", e.written())));
            concerns.push(Section {
                concern,
                files,
                constructors: constructors(&sources, concern.roots)?,
                exports: exports.iter().map(|e| e.written()).collect(),
                consumers: consumers(&imports, &member.name, &reach),
            });
        }
    }
    unrooted.sort();
    render(&concerns, &unrooted)
}

struct Section {
    concern: &'static Concern,
    files: Vec<String>,
    exports: Vec<String>,
    constructors: Vec<String>,
    /// Consuming crate, its non-test import count, and its test-only one.
    consumers: Vec<(String, usize, usize)>,
}

/// A file's module path below the crate root, as an importer writes it.
fn module_path(root: &str, file: &str) -> Option<Vec<String>> {
    let dir = root.rsplit_once('/')?.0;
    let rest = file
        .strip_prefix(dir)?
        .strip_prefix('/')?
        .strip_suffix(".rs")?;
    let rest = rest.strip_suffix("/mod").unwrap_or(rest);
    Some(rest.split('/').map(str::to_string).collect())
}

/// The `pub fn`s in a persistence module that return the type they sit on.
fn constructors(sources: &BTreeMap<String, String>, roots: &[&str]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for text in sources
        .iter()
        .filter(|(file, _)| covered(roots, file))
        .map(|(_, text)| text)
    {
        for item in syn::parse_file(text)?.items {
            let syn::Item::Impl(block) = item else {
                continue;
            };
            let Some(on) = crate::public_api::named(&block.self_ty).first().cloned() else {
                continue;
            };
            for method in block.items.iter().filter_map(|i| match i {
                syn::ImplItem::Fn(f) if matches!(f.vis, syn::Visibility::Public(_)) => Some(f),
                _ => None,
            }) {
                let syn::ReturnType::Type(_, returns) = &method.sig.output else {
                    continue;
                };
                if crate::public_api::named(returns)
                    .iter()
                    .any(|name| *name == on || name == "Self")
                {
                    out.push(format!("{on}::{}", method.sig.ident));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Every `use` in the workspace, resolved to the canonical path it names.
///
/// Resolution follows re-export chains, so a consumer importing a store through
/// another crate's facade is counted against the crate that declares it.
fn resolved_imports(ws: &Workspace) -> Result<Vec<Consumer>> {
    let index = ws.index()?;
    Ok(index
        .records
        .iter()
        .map(|record| Consumer {
            package: record.package.clone(),
            test: record.test,
            named: record
                .imports
                .iter()
                .filter_map(|import| index.resolve(record, &import.path))
                .collect(),
        })
        .filter(|c| !c.named.is_empty())
        .collect())
}

struct Consumer {
    package: String,
    test: bool,
    named: Vec<String>,
}

/// Crates outside the owner that import one of these paths, split by whether
/// the importing item is test-only.
fn consumers(imports: &[Consumer], owner: &str, reach: &[String]) -> Vec<(String, usize, usize)> {
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for consumer in imports.iter().filter(|c| c.package != owner) {
        let named = consumer
            .named
            .iter()
            .filter(|path| {
                reach
                    .iter()
                    .any(|at| *path == at || path.starts_with(&format!("{at}::")))
            })
            .count();
        if named == 0 {
            continue;
        }
        let slot = counts.entry(consumer.package.clone()).or_default();
        if consumer.test {
            slot.1 += named;
        } else {
            slot.0 += named;
        }
    }
    counts
        .into_iter()
        .map(|(package, (live, test))| (package, live, test))
        .collect()
}

fn render(sections: &[Section], unrooted: &[(String, String, String)]) -> Result<String> {
    let mut out = String::from(
        "# DAO persistence migration inventory\n\n\
         `tooling/arch-rules/src/persistence_export.rs` generates this report and \
         holds the concern table the `persistence-export` rule enforces. Regenerate it with:\n\n\
         ```bash\ncargo run -p moatless-arch-rules --bin arch-report -- --persistence-inventory\n```\n\n\
         A consumer count is the number of `use` declarations naming the concern's \
         modules or its exports, and `test` counts the ones inside a test target or a \
         test-gated item.\n\n\
         | Concern | Owner | Slice | Exports | Peer consumers |\n|---|---|---|---|---|\n",
    );
    for s in sections {
        let peers: usize = s.consumers.iter().map(|(_, live, test)| live + test).sum();
        writeln!(
            out,
            "| [{}](#{}) | {} | {:02} | {} | {peers} |",
            s.concern.name,
            s.concern.name.replace(' ', "-"),
            s.concern.owner,
            s.concern.slice,
            s.exports.len(),
        )?;
    }
    for s in sections {
        write!(out, "\n## {}\n\n", s.concern.name)?;
        let consumers = |test: bool| {
            let listed: Vec<String> = s
                .consumers
                .iter()
                .map(|(package, live, gated)| (package, if test { *gated } else { *live }))
                .filter(|(_, count)| *count > 0)
                .map(|(package, count)| format!("{package} ({count})"))
                .collect();
            if listed.is_empty() {
                "none".to_string()
            } else {
                listed.join(", ")
            }
        };
        let code = |values: &[String]| {
            if values.is_empty() {
                "none".to_string()
            } else {
                values
                    .iter()
                    .map(|v| format!("`{v}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        };
        let tables: Vec<String> = s.concern.tables.iter().map(|t| (*t).to_string()).collect();
        for (field, value) in [
            ("Owner", s.concern.owner.to_string()),
            ("Closing slice", format!("{:02}", s.concern.slice)),
            ("Persistence roots", code(&s.files)),
            ("Owning tables", code(&tables)),
            ("Exports", code(&s.exports)),
            ("Constructors", code(&s.constructors)),
            ("Peer consumers", consumers(false)),
            ("Test-only consumers", consumers(true)),
        ] {
            writeln!(out, "- **{field}:** {value}")?;
        }
    }
    out.push_str(
        "\n## Stores outside a persistence root\n\n\
         The rule reports each of these on its name alone. They sit beside the \
         concern their file names, so the slice that closes that concern owns them \
         too.\n\n\
         | Crate | Export | File |\n|---|---|---|\n",
    );
    for (package, export, file) in unrooted {
        writeln!(out, "| {package} | `{export}` | `{file}` |")?;
    }
    Ok(out)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "a fixture that fails to parse is a failing test"
)]
mod tests {
    use super::*;

    fn crate_of(files: &[(&str, &str)]) -> BTreeMap<String, String> {
        files
            .iter()
            .map(|(path, text)| ((*path).to_string(), (*text).to_string()))
            .collect()
    }

    /// Classify a hand-written crate the way [`check`] classifies a member.
    fn findings(roots: &[&str], files: &[(&str, &str)]) -> Vec<(String, &'static str)> {
        findings_publishing(roots, &[], files)
    }

    /// [`findings`] with the written paths the plan lets out one by one.
    fn findings_publishing(
        roots: &[&str],
        published: &[&str],
        files: &[(&str, &str)],
    ) -> Vec<(String, &'static str)> {
        let sources = crate_of(files);
        let surface = surface("crates/owner/src/lib.rs", &sources).unwrap();
        let exported: BTreeSet<&str> = surface.iter().map(|e| e.name.as_str()).collect();
        let layout = Layout {
            private: declared(roots, &sources)
                .unwrap()
                .into_iter()
                .filter(|name| !exported.contains(name.as_str()))
                .collect(),
            allowed: Vec::new(),
            published: published.to_vec(),
            roots: roots.to_vec(),
        };
        surface
            .iter()
            .filter_map(|e| layout.exposure(e).map(|why| (e.written(), why)))
            .collect()
    }

    const ROOTS: &[&str] = &["crates/owner/src/task/dao"];

    /// The positive control: an owner that keeps its store behind a private
    /// module and publishes an operation has nothing to report.
    #[test]
    fn owner_assembly_passes() {
        let found = findings(
            ROOTS,
            &[
                (
                    "crates/owner/src/lib.rs",
                    "pub mod task;\npub use task::tasks;",
                ),
                (
                    "crates/owner/src/task/mod.rs",
                    "mod dao;\npub struct Tasks { store: dao::PgTaskDao }\n\
                     impl Tasks { pub fn open(pool: PgPool) -> Self { todo!() } }\n\
                     pub fn tasks(pool: PgPool) -> Tasks { todo!() }",
                ),
                (
                    "crates/owner/src/task/dao.rs",
                    "pub struct PgTaskDao;\npub struct TaskRow;",
                ),
            ],
        );
        assert!(found.is_empty(), "{found:?}");
    }

    /// A foreign crate can assemble the store: the module is public, so both
    /// the store and the row it decodes are exported.
    #[test]
    fn foreign_assembly_fails() {
        let found = findings(
            ROOTS,
            &[
                ("crates/owner/src/lib.rs", "pub mod task;"),
                ("crates/owner/src/task/mod.rs", "pub mod dao;"),
                (
                    "crates/owner/src/task/dao.rs",
                    "pub struct PgTaskDao;\npub struct TaskRow;",
                ),
            ],
        );
        assert_eq!(
            found,
            vec![
                (
                    "task::dao::PgTaskDao".to_string(),
                    "a persistence module exports it"
                ),
                (
                    "task::dao::TaskRow".to_string(),
                    "a persistence module exports it"
                ),
            ]
        );
    }

    /// The exit criterion: a migrated concern cannot regain its DAO through an
    /// alias, and the alias is a key of its own rather than the old one.
    #[test]
    fn an_alias_cannot_carry_a_migrated_store_back_out() {
        let found = findings(
            ROOTS,
            &[
                (
                    "crates/owner/src/lib.rs",
                    "pub mod task;\npub use task::dao::PgTaskDao as TaskStore;",
                ),
                ("crates/owner/src/task/mod.rs", "pub mod dao;"),
                ("crates/owner/src/task/dao.rs", "pub struct PgTaskDao;"),
            ],
        );
        assert!(found.contains(&("TaskStore".to_string(), "a persistence module exports it")));
    }

    /// A private module between the store and the crate root hides nothing: the
    /// outer `pub use` still puts the store on the surface.
    #[test]
    fn a_re_export_chain_through_a_private_module_still_exposes_the_store() {
        let found = findings(
            ROOTS,
            &[
                (
                    "crates/owner/src/lib.rs",
                    "mod task;\npub use task::PgTaskDao;",
                ),
                (
                    "crates/owner/src/task/mod.rs",
                    "mod dao;\npub use dao::PgTaskDao;",
                ),
                ("crates/owner/src/task/dao.rs", "pub struct PgTaskDao;"),
            ],
        );
        assert!(found.contains(&("PgTaskDao".to_string(), "a persistence module exports it")));
    }

    /// A glob re-export is the same exposure written differently.
    #[test]
    fn a_glob_re_export_carries_the_whole_module() {
        let found = findings(
            ROOTS,
            &[
                (
                    "crates/owner/src/lib.rs",
                    "pub mod task;\npub use task::dao::*;",
                ),
                ("crates/owner/src/task/mod.rs", "pub mod dao;"),
                ("crates/owner/src/task/dao.rs", "pub struct TaskRow;"),
            ],
        );
        assert!(found.iter().any(|(path, _)| path == "TaskRow"));
    }

    /// A feature gate is not privacy: a consumer that turns `test-support` on
    /// reaches everything behind it.
    #[test]
    fn a_test_support_gate_does_not_hide_a_store() {
        let found = findings(
            ROOTS,
            &[
                (
                    "crates/owner/src/lib.rs",
                    "#[cfg(any(test, feature = \"test-support\"))]\npub mod fixtures;",
                ),
                ("crates/owner/src/fixtures.rs", "pub struct PgTaskDao;"),
            ],
        );
        assert_eq!(
            found,
            vec![("fixtures::PgTaskDao".to_string(), "its name is a store's")]
        );
    }

    /// A `#[cfg(test)]` module compiles for nobody else, so it is not exposure.
    #[test]
    fn a_unit_test_module_is_not_exposure() {
        let found = findings(
            ROOTS,
            &[
                ("crates/owner/src/lib.rs", "#[cfg(test)]\npub mod unit;"),
                ("crates/owner/src/unit.rs", "pub struct PgTaskDao;"),
            ],
        );
        assert!(found.is_empty(), "{found:?}");
    }

    /// Renaming a store does not move it out of reach: the module it sits in
    /// still decides.
    #[test]
    fn a_renamed_store_is_still_a_store() {
        let found = findings(
            ROOTS,
            &[
                ("crates/owner/src/lib.rs", "pub mod task;"),
                ("crates/owner/src/task/mod.rs", "pub mod dao;"),
                ("crates/owner/src/task/dao.rs", "pub struct TaskRepository;"),
            ],
        );
        assert_eq!(
            found,
            vec![(
                "task::dao::TaskRepository".to_string(),
                "a persistence module exports it"
            )]
        );
    }

    /// A public constructor that accepts a private store hands the store out in
    /// its signature, tuple fields and free functions included.
    #[test]
    fn a_public_signature_that_names_private_persistence_fails() {
        let found = findings(
            ROOTS,
            &[
                ("crates/owner/src/lib.rs", "pub mod task;\npub mod service;"),
                ("crates/owner/src/task/mod.rs", "mod dao;"),
                ("crates/owner/src/task/dao.rs", "pub struct TaskRow;"),
                (
                    "crates/owner/src/service.rs",
                    "pub struct Handle(pub TaskRow);\n\
                     pub fn settle(tx: &mut Transaction<'_, Postgres>) {}",
                ),
            ],
        );
        assert_eq!(
            found,
            vec![
                (
                    "service::Handle".to_string(),
                    "its public signature names private persistence"
                ),
                (
                    "service::settle".to_string(),
                    "its public signature names private persistence"
                ),
            ]
        );
    }

    /// A composition function may take the shared pool, so a pool argument is
    /// not exposure on its own.
    #[test]
    fn a_pool_argument_is_not_exposure() {
        let found = findings(
            ROOTS,
            &[(
                "crates/owner/src/lib.rs",
                "pub fn tasks(pool: PgPool) -> Tasks { todo!() }\npub struct Tasks;",
            )],
        );
        assert!(found.is_empty(), "{found:?}");
    }

    /// The shape slices 01 and 02 leave behind: the store and its constructor
    /// sit in a private persistence module, and a sibling composition function
    /// hands out the published contract over the shared pool.
    #[test]
    fn a_composition_function_beside_a_private_store_passes() {
        let found = findings_publishing(
            ROOTS,
            &["task::TaskDao"],
            &[
                (
                    "crates/owner/src/lib.rs",
                    "pub mod task;\nmod compose;\npub use compose::task_dao;",
                ),
                (
                    "crates/owner/src/task/mod.rs",
                    "mod dao;\npub use dao::TaskDao;",
                ),
                (
                    "crates/owner/src/task/dao.rs",
                    "pub trait TaskDao {}\npub(crate) struct PgTaskDao(PgPool);",
                ),
                (
                    "crates/owner/src/compose.rs",
                    "pub fn task_dao(pool: PgPool) -> Arc<dyn TaskDao> { todo!() }",
                ),
            ],
        );
        assert!(found.is_empty(), "{found:?}");
    }

    /// A composition function that hands the store back concretely publishes
    /// the store, whatever module the function sits in.
    #[test]
    fn a_composition_function_returning_the_store_fails() {
        let found = findings(
            ROOTS,
            &[
                (
                    "crates/owner/src/lib.rs",
                    "pub mod task;\nmod compose;\npub use compose::task_store;",
                ),
                ("crates/owner/src/task/mod.rs", "mod dao;"),
                ("crates/owner/src/task/dao.rs", "pub struct PgTaskDao;"),
                (
                    "crates/owner/src/compose.rs",
                    "pub fn task_store(pool: PgPool) -> PgTaskDao { todo!() }",
                ),
            ],
        );
        assert_eq!(
            found,
            vec![(
                "task_store".to_string(),
                "its public signature names private persistence"
            )]
        );
    }

    /// An enumerated operation is allowed by its own written path, and the
    /// export beside it in the same store is not.
    #[test]
    fn an_enumerated_operation_carries_no_neighbour_out() {
        let found = findings_publishing(
            ROOTS,
            &["task::dao::record_change"],
            &[
                ("crates/owner/src/lib.rs", "pub mod task;"),
                ("crates/owner/src/task/mod.rs", "pub mod dao;"),
                (
                    "crates/owner/src/task/dao.rs",
                    "pub fn record_change() {}\npub fn prune_changes() {}",
                ),
            ],
        );
        assert_eq!(
            found,
            vec![(
                "task::dao::prune_changes".to_string(),
                "a persistence module exports it"
            )]
        );
    }

    /// The checkout the two exception-list drift tests below read.
    ///
    /// [`CONCERNS`] drifts against [`root_drift`]; [`INFRASTRUCTURE`] and
    /// [`FEED_OPERATIONS`] drift when an entry stops naming something still
    /// there.
    fn workspace() -> Workspace {
        Workspace::load(crate::workspace::workspace_root()).unwrap()
    }

    #[test]
    fn every_infrastructure_path_names_a_module_on_disk() {
        let ws = workspace();
        for (owner, path) in INFRASTRUCTURE {
            assert!(
                ws.member(owner).is_some(),
                "{owner} is not a workspace member; drop its infrastructure entry"
            );
            assert!(
                ws.root.join(format!("{path}.rs")).exists() || ws.root.join(path).is_dir(),
                "{path} is gone; drop the {owner} infrastructure entry"
            );
        }
    }

    #[test]
    fn every_feed_operation_is_still_exported_under_its_written_path() {
        let ws = workspace();
        let mut by_owner: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        for (owner, _) in FEED_OPERATIONS {
            if by_owner.contains_key(owner) {
                continue;
            }
            let member = ws
                .member(owner)
                .unwrap_or_else(|| panic!("{owner} is not a workspace member"));
            let target = member
                .targets
                .iter()
                .find(|t| t.library())
                .unwrap_or_else(|| panic!("{owner} has no library"));
            let root = ws.relative(&target.path);
            let sources: BTreeMap<String, String> =
                ws.sources(member).unwrap().into_iter().collect();
            by_owner.insert(
                owner,
                surface(&root, &sources)
                    .unwrap()
                    .iter()
                    .map(Export::written)
                    .collect(),
            );
        }
        for (owner, path) in FEED_OPERATIONS {
            assert!(
                by_owner[owner].contains(*path),
                "{owner} does not export {path}; drop the feed-operation entry"
            );
        }
    }

    /// Every enumerated feed operation names a store the inventory records, so
    /// a renamed or deleted operation cannot sit here unnoticed.
    #[test]
    fn every_feed_operation_belongs_to_a_recorded_concern() {
        for (owner, path) in FEED_OPERATIONS {
            assert!(
                CONCERNS
                    .iter()
                    .any(|c| c.owner == *owner && c.roots.iter().any(|r| r.contains("change_feed"))),
                "{path} has no change-feed concern in the inventory"
            );
        }
    }

    #[test]
    fn a_module_path_covers_its_file_and_its_directory() {
        assert!(covers("a/b/dao", "a/b/dao.rs"));
        assert!(covers("a/b/dao", "a/b/dao/postgres.rs"));
        assert!(!covers("a/b/dao", "a/b/dao_tests.rs"));
        assert!(!covers("a/b/dao", "a/b/daorow.rs"));
    }

    /// Every concern names an owner that also owns its tables, and every table
    /// has one. The check runs over the real inventory rather than a fixture,
    /// because the inventory is what the slices consume.
    #[test]
    fn the_inventory_agrees_with_table_ownership() {
        assert!(table_drift().is_empty(), "{:?}", table_drift());
    }
}
