//! GitHub REST API-based fetcher for skill directories.
//!
//! Supports two layouts:
//!
//! - [`SkillLayout::Workspace`] — skills live at `{name}/SKILL.md` under either
//!   `.claude/skills` (Claude) or `.agents/skills` (Codex). Both roots are
//!   listed and merged; a skill exposed in both (typically `.claude/skills/foo`
//!   symlinking to `../../.agents/skills/foo`) is de-duplicated by directory
//!   name, with `.claude/skills` winning.
//! - [`SkillLayout::Plugin`] — plugin. Probes `.codex-plugin/plugin.json`,
//!   `.agents-plugin/plugin.json`, and `.claude-plugin/plugin.json` to resolve
//!   the skills directory, defaulting to `skills/` at the repo root. Also
//!   supports repo-local `.agents/plugins/marketplace.json` and
//!   `.claude-plugin/marketplace.json` files with local plugin entries such as
//!   `./plugins/foo`.
//!
//! Uses `GET /repos/{owner}/{repo}/contents/{prefix}` to list skill
//! directories and `GET /repos/{owner}/{repo}/contents/{prefix}/{dir}/SKILL.md`
//! with `Accept: application/vnd.github.raw+json` to fetch each SKILL.md body.
//! A 404 on the parent directory is treated as "no skills" and returns an
//! empty vector.
//!
//! Skill entries that are symlinks (e.g. `.claude/skills/foo ->
//! ../../.agents/skills/foo`) are listed by GitHub with `type: "symlink"`,
//! and the API refuses to traverse paths through a symlinked directory
//! (404). Each symlink entry is therefore resolved via its contents object's
//! `target` field before fetching `SKILL.md` from the real path.

use std::collections::HashSet;

use async_trait::async_trait;
use futures::future::try_join_all;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tracing::{debug, warn};

use super::error::SkillError;
use crate::repository::access::parse_github_owner_repo;

/// Raw SKILL.md content plus its enclosing directory name, so callers can
/// parse frontmatter themselves and fall back to the directory name when
/// `name:` is missing.
#[derive(Debug, Clone)]
pub struct FetchedSkillFile {
    pub dir_name: String,
    pub content: String,
}

/// Layout convention used by the target repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillLayout {
    /// Workspace repo: skills live at `.claude/skills/{name}/SKILL.md`.
    Workspace,
    /// Claude Code plugin: resolve the skills directory from
    /// `.claude-plugin/plugin.json` (optional `skills` field), default
    /// `skills/` at the plugin root.
    Plugin,
}

/// Abstraction over the GitHub API call so the orchestrating service can be
/// unit-tested with a hand-rolled stub.
#[async_trait]
pub trait SkillFetcher: Send + Sync {
    async fn fetch_skill_files(
        &self,
        remote_url: &str,
        layout: SkillLayout,
        token: Option<&str>,
    ) -> Result<Vec<FetchedSkillFile>, SkillError>;

    /// Ask the source what version it is at now, without fetching its content.
    ///
    /// `Ok(None)` means the source cannot be asked cheaply — the caller should
    /// treat that as "changed" and fetch, rather than serving a snapshot it
    /// cannot vouch for.
    async fn fetch_version(
        &self,
        remote_url: &str,
        token: Option<&str>,
    ) -> Result<Option<String>, SkillError>;
}

/// GitHub REST API implementation of [`SkillFetcher`].
pub struct GitHubSkillFetcher {
    client: Client,
}

impl GitHubSkillFetcher {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[derive(Debug, Deserialize)]
struct ContentEntry {
    name: String,
    #[serde(rename = "type")]
    entry_type: String,
}

/// Contents API object for a single symlink path
/// (`Accept: application/vnd.github.object+json`). Only the link target is
/// needed; directory listings include the field as `null`.
#[derive(Debug, Deserialize)]
struct SymlinkObject {
    #[serde(default)]
    target: Option<String>,
}

/// Subset of plugin manifests we need: only the optional `skills` path. All
/// other fields are ignored.
#[derive(Debug, Deserialize)]
struct PluginManifest {
    #[serde(default)]
    skills: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PluginMarketplace {
    #[serde(default)]
    plugins: Vec<PluginMarketplaceEntry>,
}

#[derive(Debug, Deserialize)]
struct PluginMarketplaceEntry {
    source: PluginMarketplaceSource,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PluginMarketplaceSource {
    LocalPath(String),
    Descriptor {
        source: String,
        #[serde(default)]
        path: Option<String>,
    },
}

/// Workspace skill roots, in precedence order. Both are listed and merged so a
/// repo that stores its real skills under `.agents/skills` (Codex) is picked up
/// whether or not it also exposes them via `.claude/skills` (Claude) symlinks.
const WORKSPACE_SKILL_PREFIXES: [&str; 2] = [".claude/skills", ".agents/skills"];

/// Plugin manifest paths, in precedence order. `.codex-plugin` is the Codex
/// repo-local plugin format; `.claude-plugin` remains supported for existing
/// Claude Code plugins.
const PLUGIN_MANIFEST_PATHS: [&str; 3] = [
    ".codex-plugin/plugin.json",
    ".agents-plugin/plugin.json",
    ".claude-plugin/plugin.json",
];

const PLUGIN_MARKETPLACE_PATHS: [&str; 2] = [
    ".agents/plugins/marketplace.json",
    ".claude-plugin/marketplace.json",
];

#[async_trait]
impl SkillFetcher for GitHubSkillFetcher {
    async fn fetch_skill_files(
        &self,
        remote_url: &str,
        layout: SkillLayout,
        token: Option<&str>,
    ) -> Result<Vec<FetchedSkillFile>, SkillError> {
        let lower = remote_url.to_ascii_lowercase();
        if !lower.contains("github.com") {
            return Err(SkillError::UnsupportedProvider {
                git_url: remote_url.to_string(),
            });
        }

        let (owner, repo) =
            parse_github_owner_repo(remote_url).ok_or_else(|| SkillError::Fetch {
                git_url: remote_url.to_string(),
                message: "cannot parse owner/repo from URL".to_string(),
            })?;

        match layout {
            SkillLayout::Workspace => {
                let mut groups: Vec<Vec<FetchedSkillFile>> = Vec::new();
                for prefix in WORKSPACE_SKILL_PREFIXES {
                    match self
                        .fetch_skills_at_prefix(&owner, &repo, prefix, token, remote_url, true)
                        .await
                    {
                        Ok(Some(files)) => groups.push(files),
                        Ok(None) => {
                            debug!(remote_url, prefix, "No skills directory on remote");
                        }
                        Err(err) if can_skip_skill_fetch_error(&err) => {
                            warn!(
                                git_url = %remote_url,
                                skills_prefix = %prefix,
                                error = %err,
                                "Skipping unreadable workspace skills directory"
                            );
                        }
                        Err(err) => return Err(err),
                    }
                }
                Ok(dedup_by_dir_name(groups))
            }
            SkillLayout::Plugin => {
                let prefixes =
                    resolve_plugin_skills_prefixes(&self.client, &owner, &repo, token, remote_url)
                        .await?;
                let mut groups: Vec<Vec<FetchedSkillFile>> = Vec::new();
                for prefix in prefixes {
                    if let Some(files) = self
                        .fetch_skills_at_prefix(&owner, &repo, &prefix, token, remote_url, false)
                        .await?
                    {
                        groups.push(files);
                    } else {
                        warn!(
                            git_url = %remote_url,
                            skills_prefix = %prefix,
                            "Plugin has no skills directory — skipping"
                        );
                    }
                }
                Ok(dedup_by_dir_name(groups))
            }
        }
    }

    /// The default branch's head commit SHA, via the commits endpoint with
    /// `Accept: application/vnd.github.sha` — one request returning 40 bytes.
    async fn fetch_version(
        &self,
        remote_url: &str,
        token: Option<&str>,
    ) -> Result<Option<String>, SkillError> {
        let lower = remote_url.to_ascii_lowercase();
        if !lower.contains("github.com") {
            return Ok(None);
        }
        let Some((owner, repo)) = parse_github_owner_repo(remote_url) else {
            return Ok(None);
        };

        let url = format!("https://api.github.com/repos/{owner}/{repo}/commits/HEAD");
        let sha = fetch_github_text(
            &self.client,
            &url,
            "application/vnd.github.sha",
            token,
            remote_url,
        )
        .await?;

        Ok(sha.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
    }
}

impl GitHubSkillFetcher {
    /// List one skills root (`skills_prefix`) and fetch every skill's `SKILL.md`.
    /// Returns `Ok(None)` when the root directory itself is absent (404), so the
    /// caller can distinguish "no such root" from "root present but empty".
    async fn fetch_skills_at_prefix(
        &self,
        owner: &str,
        repo: &str,
        skills_prefix: &str,
        token: Option<&str>,
        remote_url: &str,
        skip_unreadable_entries: bool,
    ) -> Result<Option<Vec<FetchedSkillFile>>, SkillError> {
        let list_url =
            format!("https://api.github.com/repos/{owner}/{repo}/contents/{skills_prefix}");
        let Some(entries) = send_github_json::<Vec<ContentEntry>>(
            &self.client,
            &list_url,
            "application/vnd.github+json",
            token,
            remote_url,
        )
        .await?
        else {
            return Ok(None);
        };

        let fetches = entries
            .into_iter()
            .filter(|e| e.entry_type == "dir" || e.entry_type == "symlink")
            .map(|entry| {
                let owner = owner.to_string();
                let repo = repo.to_string();
                let remote_url = remote_url.to_string();
                let client = self.client.clone();
                let token = token.map(str::to_string);
                let skills_prefix = skills_prefix.to_string();
                async move {
                    let is_symlink = entry.entry_type == "symlink";
                    let dir_name = entry.name;
                    let dir_path = if is_symlink {
                        match resolve_symlink_dir(
                            &client,
                            &owner,
                            &repo,
                            &skills_prefix,
                            &dir_name,
                            token.as_deref(),
                            &remote_url,
                        )
                        .await
                        {
                            Ok(Some(path)) => path,
                            Ok(None) => return Ok(None),
                            Err(err)
                                if should_skip_skill_entry_fetch_error(
                                    &err,
                                    skip_unreadable_entries,
                                ) =>
                            {
                                debug!(
                                    git_url = %remote_url,
                                    dir_name = %dir_name,
                                    error = %err,
                                    "Skipping unreadable symlinked skill directory"
                                );
                                return Ok(None);
                            }
                            Err(err) => return Err(err),
                        }
                    } else {
                        format!("{skills_prefix}/{dir_name}")
                    };
                    let file_url = format!(
                        "https://api.github.com/repos/{owner}/{repo}/contents/{dir_path}/SKILL.md"
                    );
                    let content =
                        match fetch_github_raw(&client, &file_url, token.as_deref(), &remote_url)
                            .await
                        {
                            Ok(content) => content,
                            Err(err)
                                if should_skip_skill_entry_fetch_error(
                                    &err,
                                    skip_unreadable_entries,
                                ) =>
                            {
                                if is_symlink {
                                    debug!(
                                        git_url = %remote_url,
                                        dir_name = %dir_name,
                                        error = %err,
                                        "Skipping unreadable symlinked skill file"
                                    );
                                } else {
                                    warn!(
                                        git_url = %remote_url,
                                        dir_name = %dir_name,
                                        error = %err,
                                        "Skipping unreadable skill file"
                                    );
                                }
                                return Ok(None);
                            }
                            Err(err) => return Err(err),
                        };
                    Ok::<Option<FetchedSkillFile>, SkillError>(
                        content.map(|content| FetchedSkillFile { dir_name, content }),
                    )
                }
            });

        let results: Vec<Option<FetchedSkillFile>> = try_join_all(fetches).await?;
        Ok(Some(results.into_iter().flatten().collect()))
    }
}

fn can_skip_skill_fetch_error(err: &SkillError) -> bool {
    matches!(err, SkillError::Fetch { .. })
}

fn should_skip_skill_entry_fetch_error(err: &SkillError, skip_unreadable_entries: bool) -> bool {
    skip_unreadable_entries && can_skip_skill_fetch_error(err)
}

/// Merge skill-file groups in priority order, keeping the first occurrence of
/// each `dir_name`. Collapses the symlink duplicates a repo exposes when the
/// same skill appears under both `.claude/skills` and `.agents/skills`.
fn dedup_by_dir_name(groups: Vec<Vec<FetchedSkillFile>>) -> Vec<FetchedSkillFile> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut merged: Vec<FetchedSkillFile> = Vec::new();
    for group in groups {
        for file in group {
            if seen.insert(file.dir_name.clone()) {
                merged.push(file);
            }
        }
    }
    merged
}

/// Resolve a symlinked skill entry to the repo-relative directory it points
/// at, by reading the symlink object's `target` and resolving it against the
/// directory containing the symlink. Returns `Ok(None)` when the target is
/// missing, absolute, or escapes the repo root — the entry is then skipped.
async fn resolve_symlink_dir(
    client: &Client,
    owner: &str,
    repo: &str,
    skills_prefix: &str,
    name: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<Option<String>, SkillError> {
    let url =
        format!("https://api.github.com/repos/{owner}/{repo}/contents/{skills_prefix}/{name}");
    let Some(link) = send_github_json::<SymlinkObject>(
        client,
        &url,
        "application/vnd.github.object+json",
        token,
        git_url,
    )
    .await?
    else {
        return Ok(None);
    };
    let Some(target) = link.target else {
        return Ok(None);
    };
    let resolved = resolve_relative_path(skills_prefix, &target);
    if resolved.is_none() {
        warn!(
            git_url,
            name, target, "Skipping symlinked skill with unresolvable target"
        );
    }
    Ok(resolved)
}

/// Resolve a symlink `target` against the directory it lives in, normalizing
/// `.` and `..` segments into a clean repo-relative path. Returns `None` for
/// absolute targets or targets that escape the repo root.
fn resolve_relative_path(base_dir: &str, target: &str) -> Option<String> {
    if target.starts_with('/') {
        return None;
    }
    let mut stack: Vec<&str> = base_dir
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                stack.pop()?;
            }
            other => stack.push(other),
        }
    }
    if stack.is_empty() {
        return None;
    }
    Some(stack.join("/"))
}

/// Resolve repo-relative skills roots for a plugin.
///
/// Supports both single-plugins and marketplace repos. Local marketplace
/// entries are resolved relative to the repository root, matching both Codex's
/// `.agents/plugins/marketplace.json` and Claude Code's
/// `.claude-plugin/marketplace.json` conventions.
async fn resolve_plugin_skills_prefixes(
    client: &Client,
    owner: &str,
    repo: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<Vec<String>, SkillError> {
    if let Some(prefix) =
        resolve_plugin_manifest_skills_prefix(client, owner, repo, "", token, git_url).await?
    {
        return Ok(vec![prefix]);
    }

    let Some((marketplace_path, raw)) =
        fetch_first_marketplace(client, owner, repo, token, git_url).await?
    else {
        return Ok(vec!["skills".to_string()]);
    };

    let marketplace: PluginMarketplace =
        serde_json::from_str(&raw).map_err(|e| SkillError::Fetch {
            git_url: git_url.to_string(),
            message: format!("invalid {marketplace_path}: {e}"),
        })?;

    let mut prefixes = Vec::new();
    for plugin in marketplace.plugins {
        let Some(path) = marketplace_local_path(&plugin.source, git_url) else {
            continue;
        };
        let plugin_root = normalize_skills_path(&path).map_err(|message| SkillError::Fetch {
            git_url: git_url.to_string(),
            message,
        })?;
        let plugin_root = plugin_root.trim_matches('/').to_string();
        let prefix = resolve_plugin_manifest_skills_prefix(
            client,
            owner,
            repo,
            &plugin_root,
            token,
            git_url,
        )
        .await?
        .unwrap_or_else(|| join_repo_paths(&plugin_root, "skills"));
        prefixes.push(prefix);
    }

    if prefixes.is_empty() {
        Ok(vec!["skills".to_string()])
    } else {
        Ok(prefixes)
    }
}

async fn fetch_first_marketplace(
    client: &Client,
    owner: &str,
    repo: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<Option<(&'static str, String)>, SkillError> {
    for marketplace_path in PLUGIN_MARKETPLACE_PATHS {
        let marketplace_url =
            format!("https://api.github.com/repos/{owner}/{repo}/contents/{marketplace_path}");
        if let Some(raw) = fetch_github_raw(client, &marketplace_url, token, git_url).await? {
            return Ok(Some((marketplace_path, raw)));
        }
    }
    Ok(None)
}

fn marketplace_local_path(source: &PluginMarketplaceSource, git_url: &str) -> Option<String> {
    match source {
        PluginMarketplaceSource::LocalPath(path) => {
            if path.trim().starts_with("./") {
                Some(path.clone())
            } else {
                debug!(
                    git_url,
                    source = %path,
                    "Skipping non-local plugin marketplace string source"
                );
                None
            }
        }
        PluginMarketplaceSource::Descriptor { source, path } if source == "local" => path.clone(),
        PluginMarketplaceSource::Descriptor { source, .. } => {
            debug!(
                git_url,
                source = %source,
                "Skipping non-local plugin marketplace descriptor source"
            );
            None
        }
    }
}

async fn resolve_plugin_manifest_skills_prefix(
    client: &Client,
    owner: &str,
    repo: &str,
    plugin_root: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<Option<String>, SkillError> {
    for manifest_path in PLUGIN_MANIFEST_PATHS {
        let repo_path = join_repo_paths(plugin_root, manifest_path);
        let manifest_url =
            format!("https://api.github.com/repos/{owner}/{repo}/contents/{repo_path}");
        let Some(raw) = fetch_github_raw(client, &manifest_url, token, git_url).await? else {
            continue;
        };
        let manifest: PluginManifest =
            serde_json::from_str(&raw).map_err(|e| SkillError::Fetch {
                git_url: git_url.to_string(),
                message: format!("invalid {repo_path}: {e}"),
            })?;
        let raw_path = manifest.skills.as_deref().unwrap_or("skills");
        let skills_path = normalize_skills_path(raw_path).map_err(|message| SkillError::Fetch {
            git_url: git_url.to_string(),
            message,
        })?;
        return Ok(Some(join_repo_paths(plugin_root, &skills_path)));
    }

    Ok(None)
}

/// Normalize a `plugin.json` `skills` value into a clean GitHub API path
/// segment:
///
/// - strips an optional leading `./`
/// - strips an optional trailing `/`
/// - rejects absolute paths (`/…`) and any `..` segment
fn normalize_skills_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('/') {
        return Err("plugin.json skills path must be repo-relative".to_string());
    }
    let stripped = trimmed
        .strip_prefix("./")
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    if stripped.split('/').any(|segment| segment == "..") {
        return Err("plugin.json skills path must not contain '..' segments".to_string());
    }
    Ok(stripped.to_string())
}

fn join_repo_paths(base: &str, child: &str) -> String {
    let base = base
        .trim()
        .strip_prefix("./")
        .unwrap_or(base.trim())
        .trim_matches('/');
    let child = child
        .trim()
        .strip_prefix("./")
        .unwrap_or(child.trim())
        .trim_matches('/');
    match (base.is_empty(), child.is_empty()) {
        (true, true) => String::new(),
        (true, false) => child.to_string(),
        (false, true) => base.to_string(),
        (false, false) => format!("{base}/{child}"),
    }
}

/// GET a GitHub Contents API URL and parse the body as JSON.
///
/// Returns `Ok(None)` on 404, `Err(SkillError::AuthRequired)` on 401/403,
/// `Err(SkillError::Fetch)` on any other non-success status or transport
/// failure.
async fn send_github_json<T: for<'de> Deserialize<'de>>(
    client: &Client,
    url: &str,
    accept: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<Option<T>, SkillError> {
    let mut req = client
        .get(url)
        .header("User-Agent", "moatless-vibe-backend")
        .header("Accept", accept);
    if let Some(tok) = token {
        req = req.header("Authorization", format!("Bearer {tok}"));
    }

    let resp = req.send().await.map_err(|e| SkillError::Fetch {
        git_url: git_url.to_string(),
        message: format!("GitHub API request failed: {e}"),
    })?;

    let status = resp.status();
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(SkillError::AuthRequired {
            git_url: git_url.to_string(),
        });
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(SkillError::Fetch {
            git_url: git_url.to_string(),
            message: format!("GitHub API returned {status}: {body}"),
        });
    }

    let parsed = resp.json::<T>().await.map_err(|e| SkillError::Fetch {
        git_url: git_url.to_string(),
        message: format!("failed to decode GitHub JSON response: {e}"),
    })?;
    Ok(Some(parsed))
}

/// GET a GitHub file with `Accept: application/vnd.github.raw+json` and
/// return its body as a string. Returns `Ok(None)` on 404.
async fn fetch_github_raw(
    client: &Client,
    url: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<Option<String>, SkillError> {
    fetch_github_text(
        client,
        url,
        "application/vnd.github.raw+json",
        token,
        git_url,
    )
    .await
}

/// GET a GitHub URL and return the body as text, or `None` on 404.
async fn fetch_github_text(
    client: &Client,
    url: &str,
    accept: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<Option<String>, SkillError> {
    let mut req = client
        .get(url)
        .header("User-Agent", "moatless-vibe-backend")
        .header("Accept", accept);
    if let Some(tok) = token {
        req = req.header("Authorization", format!("Bearer {tok}"));
    }

    let resp = req.send().await.map_err(|e| SkillError::Fetch {
        git_url: git_url.to_string(),
        message: format!("GitHub API request failed: {e}"),
    })?;

    let status = resp.status();
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(SkillError::AuthRequired {
            git_url: git_url.to_string(),
        });
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(SkillError::Fetch {
            git_url: git_url.to_string(),
            message: format!("GitHub API returned {status}: {body}"),
        });
    }

    let body = resp.text().await.map_err(|e| SkillError::Fetch {
        git_url: git_url.to_string(),
        message: format!("failed to read GitHub response body: {e}"),
    })?;
    Ok(Some(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubFetcher {
        files: Vec<FetchedSkillFile>,
    }

    #[async_trait]
    impl SkillFetcher for StubFetcher {
        async fn fetch_skill_files(
            &self,
            _remote_url: &str,
            _layout: SkillLayout,
            _token: Option<&str>,
        ) -> Result<Vec<FetchedSkillFile>, SkillError> {
            Ok(self.files.clone())
        }

        async fn fetch_version(
            &self,
            _remote_url: &str,
            _token: Option<&str>,
        ) -> Result<Option<String>, SkillError> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn stub_returns_injected_files() {
        let stub = StubFetcher {
            files: vec![FetchedSkillFile {
                dir_name: "alpha".into(),
                content: "---\nname: alpha\ndescription: A\n---\n".into(),
            }],
        };
        let files = stub
            .fetch_skill_files("https://github.com/x/y.git", SkillLayout::Workspace, None)
            .await
            .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].dir_name, "alpha");
    }

    #[test]
    fn dedup_by_dir_name_merges_claude_and_agents_prefixes() {
        let file = |dir: &str, body: &str| FetchedSkillFile {
            dir_name: dir.into(),
            content: body.into(),
        };
        // `.claude/skills` first (wins on conflict), then `.agents/skills`.
        let claude = vec![file("foo", "claude-foo"), file("shared", "claude-shared")];
        let agents = vec![file("shared", "agents-shared"), file("bar", "agents-bar")];

        let merged = dedup_by_dir_name(vec![claude, agents]);
        let names: Vec<&str> = merged.iter().map(|f| f.dir_name.as_str()).collect();
        assert_eq!(names, vec!["foo", "shared", "bar"]);
        // The `.claude/skills` copy of the shared skill wins.
        let shared = merged.iter().find(|f| f.dir_name == "shared").unwrap();
        assert_eq!(shared.content, "claude-shared");
    }

    #[tokio::test]
    async fn non_github_url_is_unsupported() {
        let fetcher = GitHubSkillFetcher::new(Client::new());
        let err = fetcher
            .fetch_skill_files(
                "https://gitlab.com/foo/bar.git",
                SkillLayout::Workspace,
                None,
            )
            .await
            .unwrap_err();
        match err {
            SkillError::UnsupportedProvider { git_url } => {
                assert_eq!(git_url, "https://gitlab.com/foo/bar.git");
            }
            other => panic!("expected UnsupportedProvider, got {other:?}"),
        }
    }

    #[test]
    fn only_workspace_best_effort_reads_skip_fetch_errors() {
        let malformed_response = SkillError::Fetch {
            git_url: "https://github.com/x/y".to_string(),
            message: "failed to decode GitHub JSON response".to_string(),
        };
        assert!(can_skip_skill_fetch_error(&malformed_response));
        assert!(should_skip_skill_entry_fetch_error(
            &malformed_response,
            true
        ));
        assert!(!should_skip_skill_entry_fetch_error(
            &malformed_response,
            false
        ));

        let auth_required = SkillError::AuthRequired {
            git_url: "https://github.com/x/y".to_string(),
        };
        assert!(!can_skip_skill_fetch_error(&auth_required));
        assert!(!should_skip_skill_entry_fetch_error(&auth_required, true));
    }

    #[test]
    fn normalize_skills_path_strips_prefix_and_trailing_slash() {
        assert_eq!(
            normalize_skills_path("./custom/skills/").unwrap(),
            "custom/skills"
        );
        assert_eq!(
            normalize_skills_path("./plugins/moatless-skills").unwrap(),
            "plugins/moatless-skills"
        );
        assert_eq!(normalize_skills_path("skills").unwrap(), "skills");
        assert_eq!(normalize_skills_path("./skills").unwrap(), "skills");
        assert_eq!(normalize_skills_path("").unwrap(), "");
        assert_eq!(
            normalize_skills_path("  custom/skills  ").unwrap(),
            "custom/skills"
        );

        let err = normalize_skills_path("/abs/path").unwrap_err();
        assert!(err.contains("repo-relative"), "got: {err}");

        let err = normalize_skills_path("../escape").unwrap_err();
        assert!(err.contains("'..'"), "got: {err}");

        let err = normalize_skills_path("custom/../skills").unwrap_err();
        assert!(err.contains("'..'"), "got: {err}");
    }

    #[test]
    fn resolve_relative_path_handles_parent_segments() {
        // The shape used by this repo: .claude/skills/foo -> ../../.agents/skills/foo
        assert_eq!(
            resolve_relative_path(".claude/skills", "../../.agents/skills/code-review"),
            Some(".agents/skills/code-review".to_string())
        );
        // Sibling target within the same directory.
        assert_eq!(
            resolve_relative_path(".claude/skills", "other-skill"),
            Some(".claude/skills/other-skill".to_string())
        );
        // `.` segments and duplicate slashes are normalized away.
        assert_eq!(
            resolve_relative_path(".claude/skills", ".//../skills-lib/./foo"),
            Some(".claude/skills-lib/foo".to_string())
        );
    }

    #[test]
    fn resolve_relative_path_rejects_escapes() {
        // Escapes the repo root.
        assert_eq!(
            resolve_relative_path(".claude/skills", "../../../outside"),
            None
        );
        // Absolute targets are not repo-relative.
        assert_eq!(resolve_relative_path(".claude/skills", "/etc/passwd"), None);
        // Resolving to the repo root itself is not a skill directory.
        assert_eq!(resolve_relative_path(".claude/skills", "../.."), None);
    }

    #[test]
    fn join_repo_paths_handles_empty_segments() {
        assert_eq!(
            join_repo_paths("plugins/moatless-skills", "skills"),
            "plugins/moatless-skills/skills"
        );
        assert_eq!(join_repo_paths("", "skills"), "skills");
        assert_eq!(
            join_repo_paths("plugins/moatless-skills/", "./skills"),
            "plugins/moatless-skills/skills"
        );
    }

    #[test]
    fn symlink_object_deserialises_target() {
        let json = r#"{
            "name": "code-review",
            "path": ".claude/skills/code-review",
            "type": "symlink",
            "target": "../../.agents/skills/code-review"
        }"#;
        let link: SymlinkObject = serde_json::from_str(json).unwrap();
        assert_eq!(
            link.target.as_deref(),
            Some("../../.agents/skills/code-review")
        );

        // Directory objects carry `"target": null` — must not fail to parse.
        let dir = r#"{"name": "skills", "type": "dir", "target": null}"#;
        let link: SymlinkObject = serde_json::from_str(dir).unwrap();
        assert!(link.target.is_none());
    }

    #[test]
    fn plugin_manifest_deserialises_spec_example() {
        // Sample manifest mirroring the shape documented at
        // https://code.claude.com/docs/en/plugins-reference — only the
        // `skills` field is meaningful to us, everything else is ignored.
        let json = r#"{
            "name": "example-plugin",
            "version": "0.1.0",
            "description": "Example plugin",
            "author": {"name": "Anthropic"},
            "skills": "./custom/skills/",
            "commands": "./custom/commands/",
            "agents": "./custom/agents/",
            "hooks": "./hooks/hooks.json"
        }"#;
        let manifest: PluginManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.skills.as_deref(), Some("./custom/skills/"));

        // Manifest without `skills` deserialises to None (default path used).
        let minimal = r#"{"name": "p", "version": "0.1.0"}"#;
        let manifest: PluginManifest = serde_json::from_str(minimal).unwrap();
        assert!(manifest.skills.is_none());
    }

    #[test]
    fn agents_marketplace_deserialises_local_plugin_entry() {
        let json = r#"{
            "name": "moatless",
            "interface": {"displayName": "Moatless"},
            "plugins": [
                {
                    "name": "moatless-skills",
                    "source": {
                        "source": "local",
                        "path": "./plugins/moatless-skills"
                    },
                    "policy": {
                        "installation": "AVAILABLE",
                        "authentication": "ON_INSTALL"
                    },
                    "category": "Development"
                }
            ]
        }"#;
        let marketplace: PluginMarketplace = serde_json::from_str(json).unwrap();
        assert_eq!(marketplace.plugins.len(), 1);
        assert_eq!(
            marketplace_local_path(&marketplace.plugins[0].source, "https://github.com/x/y")
                .as_deref(),
            Some("./plugins/moatless-skills")
        );
    }

    #[test]
    fn claude_marketplace_deserialises_string_local_plugin_entry() {
        let json = r#"{
            "$schema": "https://anthropic.com/claude-code/marketplace.schema.json",
            "name": "local",
            "plugins": [
                {
                    "name": "moatless-skills",
                    "description": "General development workflows",
                    "author": {"name": "Albert Örwall"},
                    "source": "./plugins/moatless-skills",
                    "category": "development"
                }
            ]
        }"#;
        let marketplace: PluginMarketplace = serde_json::from_str(json).unwrap();
        assert_eq!(marketplace.plugins.len(), 1);
        assert_eq!(
            marketplace_local_path(&marketplace.plugins[0].source, "https://github.com/x/y")
                .as_deref(),
            Some("./plugins/moatless-skills")
        );
    }
}
