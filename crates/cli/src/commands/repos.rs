//! Repository commands: list, get, create, update.
//!
//! A repository is only a connection to a git repo — run-configuration lives on
//! the workspace (synced from the repo's `.moatless/`), so these commands set
//! only git-connection fields.

use clap::Subcommand;
use moatless_api_schemas::enums::{RepositoryKind, RepositoryProvider, Scope};
use moatless_api_schemas::repository::{
    CreateRepositoryRequest, RepositoryResponse, UpdateRepositoryRequest,
};
use moatless_types::repository::{GitHostRepo, is_ssh_url};
use moatless_types::workspace::{derive_mount_name, proxy_clone_url, sanitize_mount_name};

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum RepoAction {
    /// List repositories
    List,
    /// Get repository details
    Get { repo_id: String },
    /// Clone a repository by id
    Clone {
        repo_id: String,
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
        /// Clone URL. HTTPS (<https://github.com/org/repo>) or SSH
        /// (git@host:org/repo.git or ssh://…). SSH remotes authenticate with an
        /// `ssh_key` secret materialized into the sandbox ~/.ssh — no provider token.
        /// Optional — a repository need not have a remote to clone (a template
        /// starter has none); the backend imposes no per-kind requirement.
        #[arg(long)]
        git_url: Option<String>,
        #[arg(long, default_value = "main")]
        branch: String,
        /// Hosting provider (github, gitlab, gitness, forgejo). Ignored for SSH clone URLs,
        /// which authenticate via an `ssh_key` secret rather than a provider token.
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
    /// Sync .moatless/ loops and workspaces from the remote repository.
    /// Warning: this is the explicit sync, so it also reclaims workspaces that
    /// were overridden from the UI/CLI, replacing their local changes.
    SyncConfig { repo_id: String },
    /// Delete a repository (soft-delete — marks as deleted, preserves history)
    Delete {
        repo_id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

pub async fn run(client: &MoatClient, action: RepoAction, json: bool) -> Result<(), ClientError> {
    match action {
        RepoAction::List => {
            let repos = client.api().list_repositories().await?;
            if json {
                output::print_json_typed(&repos);
            } else {
                format_repo_list(&repos);
            }
        }
        RepoAction::Get { repo_id } => {
            let repo = client.api().get_repository(&repo_id).await?;
            if json {
                output::print_json_typed(&repo);
            } else {
                format_repo_detail(&repo);
            }
        }
        RepoAction::Clone {
            repo_id,
            dir,
            branch,
        } => {
            let repo = client.api().get_repository(&repo_id).await?;
            let url = clone_url(client.base_url(), &repo)?;
            let target = dir.unwrap_or_else(|| checkout_dir(&repo));
            let branch = branch.or_else(|| repo.default_branch.clone());
            git_clone(&url, &target, branch.as_deref())?;
            if json {
                output::print_json_typed(&serde_json::json!({
                    "id": repo.id,
                    "name": repo.name,
                    "path": target,
                }));
            } else {
                println!("Cloned {} into {target}", repo.name);
                if let Some(gh_repo) = gh_repo_hint(&repo, &url) {
                    println!(
                        "Pass `-R {gh_repo}` to `moat gh`: origin is the backend git proxy, not a GitHub remote."
                    );
                }
            }
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
            let req = CreateRepositoryRequest {
                name,
                kind: parse_repo_kind(&kind),
                remote_url: git_url,
                default_branch: Some(branch),
                provider: parse_repo_provider(&provider),
                description,
                scope: scope.as_deref().and_then(parse_scope),
            };
            let created = client.api().create_repository(&req).await?;
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Repository created: {}", created.id);
            }
        }
        RepoAction::Update {
            repo_id,
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
            let req = UpdateRepositoryRequest {
                name,
                scope: parsed_scope,
                default_branch: branch,
                remote_url: git_url,
                description,
            };
            let updated = client.api().update_repository(&repo_id, &req).await?;
            if json {
                output::print_json_typed(&updated);
            } else {
                println!("Repository {} updated.", updated.id);
            }
        }
        RepoAction::SyncConfig { repo_id } => {
            let result = client.api().sync_config(&repo_id).await?;
            if json {
                output::print_json_typed(&result);
            } else {
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
                        "  warning: {} overridden workspace(s) reclaimed from git; local changes replaced.",
                        workspaces.reclaimed
                    );
                }
            }
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
            if json {
                output::print_json_typed(&serde_json::json!({
                    "id": id,
                    "deleted": true,
                }));
            } else {
                println!("Deleted repository {name} ({id})");
            }
        }
    }
    Ok(())
}

/// The remote `git clone` is pointed at.
///
/// Every repository whose row names a provider clones through the backend git
/// proxy, which resolves the credential from the Workspace's run-as identity: no
/// provider token has to reach the pod at all, and a Forgejo one never does. The
/// task token travels as the HTTP Basic password, supplied by the pod's
/// `credential.<backend>.helper`, so this command runs inside a sandbox.
///
/// An SSH remote clones from its own host instead. It authenticates with the
/// pod's `ssh_key` secret, and the proxy has only a token to present.
fn clone_url(base_url: &str, repo: &RepositoryResponse) -> Result<String, ClientError> {
    let remote_url = repo.remote_url.clone().ok_or_else(|| {
        ClientError::Other(format!(
            "Repository {} ({}) has no clone URL",
            repo.name, repo.id
        ))
    })?;
    if repo.provider.is_some() && !is_ssh_url(&remote_url) {
        return Ok(proxy_clone_url(base_url, &repo.id));
    }
    Ok(remote_url)
}

/// The `owner/repo` a `moat gh` command run in this checkout has to be given,
/// or `None` when its origin already names one.
///
/// `gh::resolve_repo` reads `git remote get-url origin`, and a proxied origin is
/// the backend's `/git/{id}` path.
fn gh_repo_hint(repo: &RepositoryResponse, cloned_from: &str) -> Option<String> {
    let remote_url = repo.remote_url.as_deref()?;
    if cloned_from == remote_url {
        return None;
    }
    let parsed = GitHostRepo::parse(RepositoryProvider::Github, remote_url)?;
    Some(format!("{}/{}", parsed.owner(), parsed.name()))
}

/// The directory a clone lands in.
///
/// Derived from the remote and not from the URL git is given: a proxied URL
/// ends in the repository id, so git would name the checkout after a UUID.
fn checkout_dir(repo: &RepositoryResponse) -> String {
    match repo.remote_url.as_deref() {
        Some(remote_url) => derive_mount_name(remote_url),
        None => sanitize_mount_name(&repo.name).unwrap_or_else(|| "repo".to_string()),
    }
}

/// Run `git clone`, with git's own progress on this process's stderr.
///
/// `GIT_TERMINAL_PROMPT=0` turns a missing credential into an error rather than
/// a prompt no agent shell answers.
fn git_clone(url: &str, target: &str, branch: Option<&str>) -> Result<(), ClientError> {
    let mut command = std::process::Command::new("git");
    command.env("GIT_TERMINAL_PROMPT", "0").arg("clone");
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
        "gitness" => Some(RepositoryProvider::Gitness),
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
    println!("Branch:   {}", r.default_branch.as_deref().unwrap_or("-"));
    println!("Git URL:  {}", r.remote_url.as_deref().unwrap_or("-"));
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

    const REPO_ID: &str = "6a12f676-2716-4de8-9bcc-8889e7f7fe3c";
    const BASE_URL: &str = "https://moatless.dev";

    fn repository(
        provider: Option<RepositoryProvider>,
        remote_url: Option<&str>,
    ) -> RepositoryResponse {
        RepositoryResponse {
            id: REPO_ID.to_string(),
            name: "config default".to_string(),
            kind: RepositoryKind::Git,
            remote_url: remote_url.map(str::to_string),
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
        format!("https://moatless.dev/git/{REPO_ID}")
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

    #[test]
    fn an_ssh_remote_clones_from_its_own_host() {
        let remote = "ssh://git@git.example.com:2222/owner/repo.git";
        let repo = repository(Some(RepositoryProvider::Gitness), Some(remote));
        assert_eq!(clone_url(BASE_URL, &repo).unwrap(), remote);
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
    fn a_proxied_github_checkout_reports_the_repository_gh_needs() {
        let repo = repository(
            Some(RepositoryProvider::Github),
            Some("https://github.com/owner/repo.git"),
        );
        assert_eq!(
            gh_repo_hint(&repo, &proxied()).as_deref(),
            Some("owner/repo")
        );
    }

    #[test]
    fn a_checkout_gh_can_read_itself_reports_nothing() {
        let remote = "https://github.com/owner/repo.git";
        let repo = repository(Some(RepositoryProvider::Github), Some(remote));
        assert_eq!(gh_repo_hint(&repo, remote), None);

        let forgejo = repository(
            Some(RepositoryProvider::Forgejo),
            Some("https://git.example.com/owner/repo.git"),
        );
        assert_eq!(gh_repo_hint(&forgejo, &proxied()), None);
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
}
