//! Repository commands: list, get, create, update.
//!
//! A repository is only a connection to a git repo — run-configuration lives on
//! the workspace (synced from the repo's `.moatless/`), so these commands set
//! only git-connection fields.

use clap::Subcommand;
use moatless_api_schemas::enums::{RepositoryKind, RepositoryProvider, Scope};
use moatless_api_schemas::task::AddTaskRepositoryRequest;
use moatless_workspace_types::schemas::repository::{
    CreateRepositoryRequest, RepositoryResponse, UpdateRepositoryRequest,
};
use moatless_workspace_types::workspace::{
    derive_mount_name, github_remote_config, proxy_clone_url, sanitize_mount_name,
};

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum RepoAction {
    /// List repositories
    List {
        /// Keep only repositories hosted by this provider (github, gitlab,
        /// forgejo)
        #[arg(long)]
        provider: Option<String>,
        /// Keep only repositories at this scope: global or user
        #[arg(long)]
        scope: Option<String>,
        /// Keep only repositories whose name, description or remote URL
        /// contains this text, matched case-insensitively
        #[arg(long)]
        search: Option<String>,
    },
    /// Get repository details
    Get {
        /// Repository id, or its name as `repos list` shows it
        repo: String,
    },
    /// Clone a repository by id or name
    Clone {
        /// Repository id, or its name as `repos list` shows it
        repo: String,
        /// Directory to clone into. Defaults to the repository's own name.
        dir: Option<String>,
        /// Branch to check out. Defaults to the repository's default branch.
        #[arg(long)]
        branch: Option<String>,
    },
    /// Create a repository
    Create {
        #[arg(long)]
        name: String,
        /// Clone URL over HTTPS; SSH remotes are refused. Optional — a
        /// repository need not have a remote (a template starter has none);
        /// the backend imposes no per-kind requirement.
        #[arg(long)]
        git_url: Option<String>,
        #[arg(long, default_value = "main")]
        branch: String,
        /// Hosting provider (github, gitlab, forgejo). Name the one serving
        /// the host in `--git-url`: `github` is refused for any other host,
        /// while `gitlab` and `forgejo` parse any host.
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
        provider: Option<String>,
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
    /// Register every repository a GitHub organization owns that is missing
    /// here, and fill in a description the registry lacks from the one GitHub
    /// carries. Reads the organization through the local `gh` CLI.
    Import {
        /// GitHub organization or user to read repositories from
        #[arg(long)]
        org: String,
        /// Scope to create missing repositories at: global or user
        #[arg(long, default_value = "global")]
        scope: String,
        /// Include archived repositories, which are skipped by default
        #[arg(long)]
        include_archived: bool,
        /// Report what would change and write nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Sync .moatless/ loops and workspaces from the remote repository.
    /// The backend already does this when a GitHub default branch moves.
    SyncConfig {
        /// Repository id, or its name as `repos list` shows it
        repo: String,
    },
    /// Delete a repository (soft-delete — marks as deleted, preserves history)
    Delete {
        repo_id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

/// `env_token_set` is [`Config::github_token_set`](crate::config::Config::github_token_set).
/// `task_id` is the Task this runs inside, which a clone is recorded on.
pub async fn run(
    client: &MoatClient,
    action: RepoAction,
    json: bool,
    env_token_set: bool,
    task_id: Option<&str>,
) -> Result<(), ClientError> {
    match action {
        RepoAction::List {
            provider,
            scope,
            search,
        } => {
            let filter = RepoFilter::parse(provider.as_deref(), scope.as_deref(), search)?;
            let repos = client.api().list_repositories().await?;
            let repos: Vec<RepositoryResponse> =
                repos.into_iter().filter(|r| filter.matches(r)).collect();
            output::emit(json, &repos, || format_repo_list(&repos));
        }
        RepoAction::Get { repo } => {
            let repo = resolve_repo(client, &repo).await?;
            output::emit(json, &repo, || format_repo_detail(&repo));
        }
        RepoAction::Clone { repo, dir, branch } => {
            let repo = resolve_repo(client, &repo).await?;
            let url = clone_url(client.base_url(), &repo)?;
            let target = dir.unwrap_or_else(|| checkout_dir(&repo));
            let branch = branch.or_else(|| repo.default_branch.clone());
            let config = clone_config(&repo);
            git_clone(&url, &target, branch.as_deref(), &config)?;
            if let Some(task_id) = task_id {
                // The clone already happened, so a refused record warns rather
                // than failing it.
                let recorded = client
                    .api()
                    .add_task_repository(
                        task_id,
                        &AddTaskRepositoryRequest {
                            repository_id: repo.id.clone(),
                        },
                    )
                    .await;
                if let Err(e) = recorded {
                    eprintln!(
                        "Warning: could not record {} on task {task_id}: {e}",
                        repo.name
                    );
                }
            }
            output::emit(
                json,
                &serde_json::json!({
                    "id": repo.id,
                    "name": repo.name,
                    "path": target,
                }),
                || println!("Cloned {} into {target}", repo.name),
            );
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
            // Not `parse_repo_provider(&provider)`: an unreadable value would
            // send `provider: null`, which registers the unaddressed row the
            // backend's mismatch check exists to refuse.
            let parsed_provider = parse_repo_provider(&provider)
                .ok_or_else(|| ClientError::Other(format!("Invalid --provider '{provider}'")))?;
            let req = CreateRepositoryRequest {
                name,
                kind: parse_repo_kind(&kind),
                remote_url: git_url,
                default_branch: Some(branch),
                provider: Some(parsed_provider),
                description,
                scope: scope.as_deref().and_then(parse_scope),
            };
            let created = client.api().create_repository(&req).await?;
            output::emit(json, &created, || {
                println!("Repository created: {}", created.id);
            });
        }
        RepoAction::Update {
            repo_id,
            provider,
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
            let provider = provider
                .as_deref()
                .map(|provider| {
                    parse_repo_provider(provider).ok_or_else(|| {
                        ClientError::Other(format!("Invalid --provider '{provider}'"))
                    })
                })
                .transpose()?;
            let req = UpdateRepositoryRequest {
                provider,
                name,
                scope: parsed_scope,
                default_branch: branch,
                remote_url: git_url,
                description,
            };
            let updated = client.api().update_repository(&repo_id, &req).await?;
            output::emit(json, &updated, || {
                println!("Repository {} updated.", updated.id);
            });
        }
        RepoAction::Import {
            org,
            scope,
            include_archived,
            dry_run,
        } => {
            let scope = parse_scope(&scope).ok_or_else(|| {
                ClientError::Other(format!(
                    "Invalid --scope '{scope}' (expected: global or user)"
                ))
            })?;
            let (report, outcome) = import_org(
                client,
                env_token_set,
                &org,
                scope,
                include_archived,
                dry_run,
            )
            .await;
            output::emit(json, &report, || format_import_report(&report, dry_run));
            // Report first: a failure part-way leaves the rows written so far,
            // and propagating straight out would print none of them.
            outcome?;
        }
        RepoAction::SyncConfig { repo } => {
            let repo = resolve_repo(client, &repo).await?;
            let result = client.api().sync_config(&repo.id).await?;
            output::emit(json, &result, || {
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
                        "  warning: {} manual workspace(s) reclaimed by git; their settings replaced.",
                        workspaces.reclaimed
                    );
                }
                for skip in &workspaces.skipped {
                    println!("  skipped {}: {}", skip.source_key, skip.reason);
                }
                for warning in &workspaces.warnings {
                    println!("  warning {}: {}", warning.path, warning.message);
                }
            });
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
            output::emit(
                json,
                &serde_json::json!({
                    "id": id,
                    "deleted": true,
                }),
                || println!("Deleted repository {name} ({id})"),
            );
        }
    }
    Ok(())
}

/// Resolve a repository from an id or a name.
///
/// A UUID-shaped value goes straight to the backend as an id; anything else
/// matches the registry by name, case-insensitively. A name shared by two
/// repositories is refused rather than picked between, since checking out the
/// wrong one stays silent until the build fails.
async fn resolve_repo(
    client: &MoatClient,
    id_or_name: &str,
) -> Result<RepositoryResponse, ClientError> {
    if uuid::Uuid::parse_str(id_or_name).is_ok() {
        return Ok(client.api().get_repository(id_or_name).await?);
    }
    let mut matches: Vec<RepositoryResponse> = client
        .api()
        .list_repositories()
        .await?
        .into_iter()
        .filter(|r| r.name.eq_ignore_ascii_case(id_or_name))
        .collect();
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(ClientError::Other(format!(
            "No repository named '{id_or_name}' — run `moat repos list --search {id_or_name}` to find it"
        ))),
        _ => {
            let ids = matches
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(ClientError::Other(format!(
                "'{id_or_name}' names {} repositories — use an id: {ids}",
                matches.len()
            )))
        }
    }
}

/// The remote `git clone` is pointed at.
///
/// A repository with a provider clones through the backend git proxy, using
/// the Workspace's run-as identity — no provider token reaches the pod; the
/// task token travels as the HTTP Basic password.
fn clone_url(base_url: &str, repo: &RepositoryResponse) -> Result<String, ClientError> {
    let remote = repo.remote.as_ref().ok_or_else(|| {
        ClientError::Other(format!(
            "Repository {} ({}) has no clone URL",
            repo.name, repo.id
        ))
    })?;
    if remote.host_repo.is_some() {
        return Ok(proxy_clone_url(base_url, &repo.id));
    }
    Ok(remote.url.clone())
}

/// The `git clone --config` values a clone of `repo` takes: the
/// [`github_remote_config`] for a GitHub repository, which clones through the
/// proxy and so names no GitHub repository in `origin`.
fn clone_config(repo: &RepositoryResponse) -> Vec<String> {
    let Some(remote) = repo.remote.as_ref() else {
        return Vec::new();
    };
    match &remote.host_repo {
        Some(host) if host.provider == RepositoryProvider::Github => {
            github_remote_config(&remote.url).to_vec()
        }
        _ => Vec::new(),
    }
}

/// The directory a clone lands in.
///
/// Derived from the remote, not the URL git is given — a proxied URL ends in
/// the repository id, so git would otherwise name the checkout after a UUID.
fn checkout_dir(repo: &RepositoryResponse) -> String {
    match repo.remote.as_ref().map(|remote| remote.url.as_str()) {
        Some(remote_url) => derive_mount_name(remote_url),
        None => sanitize_mount_name(&repo.name).unwrap_or_else(|| "repo".to_string()),
    }
}

/// Run `git clone`, with git's own progress on this process's stderr.
///
/// `GIT_TERMINAL_PROMPT=0` turns a missing credential into an error rather than
/// a prompt no agent shell answers.
fn git_clone(
    url: &str,
    target: &str,
    branch: Option<&str>,
    config: &[String],
) -> Result<(), ClientError> {
    let mut command = std::process::Command::new("git");
    command.env("GIT_TERMINAL_PROMPT", "0").arg("clone");
    for setting in config {
        command.args(["--config", setting]);
    }
    if let Some(branch) = branch {
        command.args(["--branch", branch]);
    }
    // `--` closes the option list: nothing validates a stored `remote_url`, so a
    // row starting one with `-` would otherwise hand `git clone` a flag.
    let status = command
        .args(["--", url, target])
        .status()
        .map_err(|e| ClientError::Other(format!("Failed to run git: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(ClientError::Other(format!("git clone exited {status}")))
    }
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
        "forgejo" => Some(RepositoryProvider::Forgejo),
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

/// One repository as `gh repo list --json` reports it.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhRepo {
    name_with_owner: String,
    name: String,
    url: String,
    description: Option<String>,
    is_archived: bool,
    default_branch_ref: Option<GhBranchRef>,
}

#[derive(serde::Deserialize)]
struct GhBranchRef {
    name: String,
}

/// What one repository in the organization needed.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportEntry {
    pub name: String,
    pub action: &'static str,
    /// The registry row, absent while `--dry-run` and on a skipped repository.
    pub id: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub org: String,
    pub created: usize,
    pub described: usize,
    pub unchanged: usize,
    pub skipped_archived: usize,
    pub skipped_name_taken: usize,
    pub entries: Vec<ImportEntry>,
}

/// `gh` caps a listing at 30 without `--limit`, which silently truncates an org
/// this size and makes the import look complete.
const GH_LIST_LIMIT: usize = 500;

/// What the import does to one organization repository.
#[derive(Debug, PartialEq, Eq)]
enum ImportPlan {
    Create,
    /// The registry carries the row without a description and GitHub has one.
    /// Holds the id of the row to fill.
    Describe(String),
    Unchanged,
    SkipArchived,
    /// A different row already answers to this name, so creating would leave
    /// `repos clone <name>` unable to resolve either one.
    SkipNameTaken,
}

fn plan_import(
    known: Option<&RepositoryResponse>,
    repo: &GhRepo,
    include_archived: bool,
    name_taken: bool,
) -> ImportPlan {
    if repo.is_archived && !include_archived {
        return ImportPlan::SkipArchived;
    }
    let Some(known) = known else {
        return if name_taken {
            ImportPlan::SkipNameTaken
        } else {
            ImportPlan::Create
        };
    };
    if known.description.is_none() && repo.description.is_some() {
        ImportPlan::Describe(known.id.clone())
    } else {
        ImportPlan::Unchanged
    }
}

async fn import_org(
    client: &MoatClient,
    env_token_set: bool,
    org: &str,
    scope: Scope,
    include_archived: bool,
    dry_run: bool,
) -> (ImportReport, Result<(), ClientError>) {
    let mut report = ImportReport {
        org: org.to_string(),
        created: 0,
        described: 0,
        unchanged: 0,
        skipped_archived: 0,
        skipped_name_taken: 0,
        entries: Vec::new(),
    };

    let remote = match fetch_org_repos(org, env_token_set) {
        Ok(remote) => remote,
        Err(e) => return (report, Err(e)),
    };
    let existing = match client.api().list_repositories().await {
        Ok(existing) => existing,
        Err(e) => return (report, Err(e.into())),
    };

    let by_key: std::collections::HashMap<_, _> = existing
        .iter()
        .filter_map(|repo| {
            let host = repo.remote.as_ref()?.host_repo.as_ref()?;
            (host.provider == RepositoryProvider::Github).then(|| {
                (
                    format!("{}/{}", host.owner, host.name).to_ascii_lowercase(),
                    repo,
                )
            })
        })
        .collect();
    let names: std::collections::HashSet<String> =
        existing.iter().map(|r| r.name.to_lowercase()).collect();

    for repo in &remote {
        let known = by_key
            .get(&repo.name_with_owner.to_ascii_lowercase())
            .copied();
        let name_taken = known.is_none() && names.contains(&repo.name.to_lowercase());
        match plan_import(known, repo, include_archived, name_taken) {
            ImportPlan::SkipArchived => report.skipped_archived += 1,
            ImportPlan::Unchanged => report.unchanged += 1,
            ImportPlan::SkipNameTaken => {
                report.skipped_name_taken += 1;
                report.entries.push(ImportEntry {
                    name: repo.name.clone(),
                    action: "name taken",
                    id: None,
                });
            }
            ImportPlan::Describe(known_id) => {
                let id = if dry_run {
                    None
                } else {
                    let req = UpdateRepositoryRequest {
                        description: repo.description.clone(),
                        ..Default::default()
                    };
                    match client.api().update_repository(&known_id, &req).await {
                        Ok(updated) => Some(updated.id),
                        Err(e) => return (report, Err(e.into())),
                    }
                };
                report.described += 1;
                report.entries.push(ImportEntry {
                    name: repo.name.clone(),
                    action: "described",
                    id,
                });
            }
            ImportPlan::Create => {
                let id = if dry_run {
                    None
                } else {
                    let req = CreateRepositoryRequest {
                        name: repo.name.clone(),
                        kind: RepositoryKind::Remote,
                        remote_url: Some(repo.url.clone()),
                        default_branch: repo
                            .default_branch_ref
                            .as_ref()
                            .map(|b| b.name.clone())
                            .or_else(|| Some("main".to_string())),
                        provider: Some(RepositoryProvider::Github),
                        description: repo.description.clone(),
                        scope: Some(scope),
                    };
                    match client.api().create_repository(&req).await {
                        Ok(created) => Some(created.id),
                        Err(e) => return (report, Err(e.into())),
                    }
                };
                report.created += 1;
                report.entries.push(ImportEntry {
                    name: repo.name.clone(),
                    action: "created",
                    id,
                });
            }
        }
    }

    (report, Ok(()))
}

fn fetch_org_repos(org: &str, env_token_set: bool) -> Result<Vec<GhRepo>, ClientError> {
    let args: Vec<String> = [
        "repo",
        "list",
        org,
        "--limit",
        &GH_LIST_LIMIT.to_string(),
        "--json",
        "name,nameWithOwner,url,description,isArchived,defaultBranchRef",
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    let token = super::gh::github_token_for_gh(env_token_set);
    let stdout = super::gh::gh_command_output(&args, token.as_deref(), None)
        .map_err(|e| ClientError::Other(e.to_string()))?;
    let mut remote: Vec<GhRepo> = serde_json::from_slice(&stdout)
        .map_err(|e| ClientError::Other(format!("could not read `gh repo list` output: {e}")))?;
    // `gh repo list` reports no total, so a listing that comes back exactly at
    // the limit is the only signal that it was cut.
    if remote.len() >= GH_LIST_LIMIT {
        eprintln!(
            "`gh repo list {org}` returned the full {GH_LIST_LIMIT}-row limit; some repositories may be missing."
        );
    }
    remote.sort_by_key(|r| r.name.to_lowercase());
    Ok(remote)
}

fn format_import_report(report: &ImportReport, dry_run: bool) {
    if report.entries.is_empty() {
        let seen = report.unchanged + report.skipped_archived;
        if seen == 0 {
            println!(
                "`gh repo list {}` returned no repositories — check the organization name.",
                report.org
            );
        } else {
            println!(
                "{} is fully registered: {} repositories, {} archived and skipped.",
                report.org, report.unchanged, report.skipped_archived
            );
        }
        return;
    }
    let rows: Vec<Vec<String>> = report
        .entries
        .iter()
        .map(|e| {
            vec![
                e.action.to_string(),
                e.name.clone(),
                e.id.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    output::print_table(&["ACTION", "NAME", "ID"], &rows);
    let verb = if dry_run { "would be" } else { "were" };
    println!(
        "{}: {} {verb} created, {} {verb} described, {} already correct, {} archived and skipped, {} skipped for a name already in use.",
        report.org,
        report.created,
        report.described,
        report.unchanged,
        report.skipped_archived,
        report.skipped_name_taken
    );
}

/// The `--provider`, `--scope` and `--search` predicates a listing applies
/// client-side; `GET /api/v1/repositories` takes no query parameters.
struct RepoFilter {
    provider: Option<RepositoryProvider>,
    scope: Option<Scope>,
    search: Option<String>,
}

impl RepoFilter {
    fn parse(
        provider: Option<&str>,
        scope: Option<&str>,
        search: Option<String>,
    ) -> Result<Self, ClientError> {
        let provider = match provider {
            Some(p) => Some(parse_repo_provider(p).ok_or_else(|| {
                ClientError::Other(format!(
                    "Invalid --provider '{p}' (expected: github, gitlab or forgejo)"
                ))
            })?),
            None => None,
        };
        let scope = match scope {
            Some(s) => Some(parse_scope(s).ok_or_else(|| {
                ClientError::Other(format!("Invalid --scope '{s}' (expected: global or user)"))
            })?),
            None => None,
        };
        Ok(Self {
            provider,
            scope,
            search: search.map(|s| s.to_lowercase()),
        })
    }

    fn matches(&self, repo: &RepositoryResponse) -> bool {
        if self.provider.is_some_and(|p| repo.provider != Some(p)) {
            return false;
        }
        if self.scope.is_some_and(|s| repo.scope != s) {
            return false;
        }
        let Some(needle) = &self.search else {
            return true;
        };
        [
            Some(repo.name.as_str()),
            repo.description.as_deref(),
            repo.remote.as_ref().map(|remote| remote.url.as_str()),
        ]
        .into_iter()
        .flatten()
        .any(|field| field.to_lowercase().contains(needle))
    }
}

/// Width the description is cut to, so a long one does not push NAME and
/// PROVIDER off a terminal. `--json` and `repos get` carry the full text.
const DESCRIPTION_WIDTH: usize = 72;

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
                r.scope.to_string(),
                output::truncate(r.description.as_deref().unwrap_or("-"), DESCRIPTION_WIDTH),
            ]
        })
        .collect();

    output::print_table(
        &["ID", "NAME", "PROVIDER", "BRANCH", "SCOPE", "DESCRIPTION"],
        &rows,
    );
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
    println!(
        "Git URL:  {}",
        r.remote.as_ref().map_or("-", |remote| remote.url.as_str())
    );
    if let Some(ref desc) = r.description {
        println!("Description: {desc}");
    }
    println!("Scope:      {}", r.scope);
    if let Some(ref owner) = r.owner_user_id {
        println!("Owner:      {owner}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_workspace_types::schemas::repository::RepositoryRemoteResponse;

    const REPO_ID: &str = "6a12f676-2716-4de8-9bcc-8889e7f7fe3c";
    const BASE_URL: &str = "https://moatless.example.com";

    fn repository(
        provider: Option<RepositoryProvider>,
        remote_url: Option<&str>,
    ) -> RepositoryResponse {
        RepositoryResponse {
            remote: remote_url.map(|url| RepositoryRemoteResponse {
                url: url.to_string(),
                host_repo: provider.map(|provider| {
                    moatless_workspace_types::schemas::repository::GitHostRepoResponse {
                        provider,
                        host: "github.com".to_string(),
                        owner: "soaplabs".to_string(),
                        name: "foo".to_string(),
                    }
                }),
            }),
            id: REPO_ID.to_string(),
            name: "config default".to_string(),
            kind: RepositoryKind::Git,
            default_branch: None,
            description: None,
            provider,
            scope: Scope::User,
            owner_user_id: None,
            created_by: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn proxied() -> String {
        format!("{BASE_URL}/git/{REPO_ID}")
    }

    #[test]
    fn a_forgejo_repository_clones_through_the_proxy() {
        let repo = repository(
            Some(RepositoryProvider::Forgejo),
            Some("https://git.example.com/owner/config-default.git"),
        );
        assert_eq!(clone_url(BASE_URL, &repo).unwrap(), proxied());
    }

    #[test]
    fn a_github_repository_clones_through_the_proxy() {
        let repo = repository(
            Some(RepositoryProvider::Github),
            Some("https://github.com/owner/repo.git"),
        );
        assert_eq!(clone_url(BASE_URL, &repo).unwrap(), proxied());
    }

    /// Without the `github` remote, plain `gh` finds no GitHub repository in
    /// the proxied `origin`.
    #[test]
    fn a_github_clone_names_its_repository_in_a_github_remote() {
        let repo = repository(
            Some(RepositoryProvider::Github),
            Some("https://github.com/owner/repo.git"),
        );
        assert_eq!(
            clone_config(&repo),
            [
                "remote.github.url=https://github.com/owner/repo.git",
                "remote.github.fetch=+refs/heads/*:refs/remotes/github/*",
                "remote.github.skipFetchAll=true",
            ]
        );
    }

    #[test]
    fn a_non_github_clone_adds_no_remote() {
        let forgejo = repository(
            Some(RepositoryProvider::Forgejo),
            Some("https://git.example.com/owner/config-default.git"),
        );
        assert!(clone_config(&forgejo).is_empty());
        assert!(
            clone_config(&repository(None, Some("https://git.example.com/o/r.git"))).is_empty()
        );
    }

    #[test]
    fn a_repository_naming_no_provider_clones_from_its_own_host() {
        let remote = "https://git.example.com/owner/repo.git";
        let repo = repository(None, Some(remote));
        assert_eq!(clone_url(BASE_URL, &repo).unwrap(), remote);
    }

    #[test]
    fn a_repository_with_no_remote_has_no_clone_url() {
        let repo = repository(Some(RepositoryProvider::Github), None);
        assert!(clone_url(BASE_URL, &repo).is_err());
    }

    #[test]
    fn a_proxied_clone_is_named_after_the_remote() {
        let repo = repository(
            Some(RepositoryProvider::Forgejo),
            Some("https://git.example.com/owner/config-default.git"),
        );
        assert_eq!(checkout_dir(&repo), "config-default");
    }

    #[test]
    fn a_repository_with_no_remote_is_named_after_itself() {
        let repo = repository(Some(RepositoryProvider::Github), None);
        assert_eq!(checkout_dir(&repo), "config-default");
    }

    fn filter(provider: Option<&str>, scope: Option<&str>, search: Option<&str>) -> RepoFilter {
        RepoFilter::parse(provider, scope, search.map(str::to_string)).expect("parses")
    }

    #[test]
    fn a_filter_naming_no_predicate_keeps_every_repository() {
        let repo = repository(Some(RepositoryProvider::Github), None);
        assert!(filter(None, None, None).matches(&repo));
    }

    #[test]
    fn provider_and_scope_narrow_independently() {
        let repo = repository(
            Some(RepositoryProvider::Forgejo),
            Some("https://git.example.com/owner/repo.git"),
        );
        assert!(filter(Some("forgejo"), None, None).matches(&repo));
        assert!(!filter(Some("github"), None, None).matches(&repo));
        assert!(filter(None, Some("user"), None).matches(&repo));
        assert!(!filter(None, Some("global"), None).matches(&repo));
    }

    #[test]
    fn search_reads_name_description_and_remote_url_case_insensitively() {
        let mut repo = repository(
            Some(RepositoryProvider::Github),
            Some("https://github.com/soaplabs/soap-backend.git"),
        );
        repo.description = Some("Core AML API".to_string());
        assert!(filter(None, None, Some("CONFIG")).matches(&repo));
        assert!(filter(None, None, Some("aml api")).matches(&repo));
        assert!(filter(None, None, Some("soaplabs")).matches(&repo));
        assert!(!filter(None, None, Some("screening")).matches(&repo));
    }

    #[test]
    fn an_unreadable_predicate_is_refused_rather_than_ignored() {
        assert!(RepoFilter::parse(Some("bitbucket"), None, None).is_err());
        assert!(RepoFilter::parse(None, Some("team"), None).is_err());
    }

    #[test]
    fn a_description_is_cut_only_when_it_is_too_long() {
        assert_eq!(output::truncate("short", 10), "short");
        assert_eq!(output::truncate("0123456789", 10), "0123456789");
        assert_eq!(output::truncate("0123456789x", 10), "012345678…");
        assert_eq!(output::truncate("0123456789x", 10).chars().count(), 10);
    }

    fn gh_repo(name: &str, description: Option<&str>, is_archived: bool) -> GhRepo {
        GhRepo {
            name_with_owner: format!("soaplabs/{name}"),
            name: name.to_string(),
            url: format!("https://github.com/soaplabs/{name}"),
            description: description.map(str::to_string),
            is_archived,
            default_branch_ref: None,
        }
    }

    #[test]
    fn an_archived_repository_is_skipped_unless_it_is_asked_for() {
        let repo = gh_repo("foo", Some("a description"), true);
        assert_eq!(
            plan_import(None, &repo, false, false),
            ImportPlan::SkipArchived
        );
        assert_eq!(plan_import(None, &repo, true, false), ImportPlan::Create);
    }

    #[test]
    fn a_description_is_filled_only_where_the_registry_has_none() {
        let mut known = repository(Some(RepositoryProvider::Github), None);
        let described = gh_repo("foo", Some("what it does"), false);

        assert_eq!(
            plan_import(Some(&known), &described, false, false),
            ImportPlan::Describe(REPO_ID.to_string())
        );

        known.description = Some("what we already say".to_string());
        assert_eq!(
            plan_import(Some(&known), &described, false, false),
            ImportPlan::Unchanged
        );

        known.description = None;
        let undescribed = gh_repo("foo", None, false);
        assert_eq!(
            plan_import(Some(&known), &undescribed, false, false),
            ImportPlan::Unchanged
        );
    }

    #[test]
    fn a_repository_whose_name_is_taken_is_reported_rather_than_duplicated() {
        let repo = gh_repo("foo", Some("a description"), false);
        assert_eq!(
            plan_import(None, &repo, false, true),
            ImportPlan::SkipNameTaken
        );
    }
}
