//! Git-host REST fetcher for skill directories, over GitHub and Forgejo.
//!
//! Two layouts — see [`SkillLayout::Workspace`] and [`SkillLayout::Plugin`].
//!
//! Lists directories via `GET .../contents/{prefix}` and fetches each
//! `SKILL.md` raw (a 404 on the parent means no skills). Symlinked entries —
//! which neither host traverses through directly — are resolved via their
//! contents object's `target` field. [`RepoApi`] is the only thing here that
//! knows which host it is addressing.

use std::collections::HashSet;

use async_trait::async_trait;
use futures::future::try_join_all;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tracing::{debug, warn};

use super::error::SkillError;
use moatless_types::http::HttpTimeouts;
use moatless_workspace_types::plugin::{
    PLUGIN_MANIFEST_PATHS, PLUGIN_MARKETPLACE_PATHS, PluginMarketplaceSource, REPO_SKILL_ROOTS,
    marketplace_local_path,
};
use moatless_workspace_types::repository::RepositoryProvider;
use moatless_workspace_types::repository::RepositoryRemote;

/// Raw SKILL.md content plus its enclosing directory name, so callers can parse
/// frontmatter themselves and fall back to the directory name when `name:` is
/// missing.
#[derive(Debug, Clone)]
pub struct FetchedSkillFile {
    pub dir_name: String,
    /// Repo-relative path of the `SKILL.md` itself, resolved through a symlink
    /// so it names the directory holding the file rather than the link. Ends up
    /// on `SkillResponse::path`, whose doc says what a reader does with it.
    pub path: String,
    pub content: String,
}

/// Layout convention used by the target repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillLayout {
    /// Workspace repo: merges `.claude/skills/{name}/SKILL.md` with
    /// `.agents/skills` (`.claude/skills` wins on a name clash).
    Workspace,
    /// Claude Code plugin: resolve the skills directory from
    /// `.claude-plugin/plugin.json` (optional `skills` field), default
    /// `skills/` at the plugin root.
    Plugin,
}

/// Abstraction over the git-host API call so the orchestrating service can be
/// unit-tested with a hand-rolled stub.
#[async_trait]
pub trait SkillFetcher: Send + Sync {
    async fn fetch_skill_files(
        &self,
        remote: &RepositoryRemote,
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
        remote: &RepositoryRemote,
        token: Option<&str>,
    ) -> Result<Option<String>, SkillError>;

    /// The whole repository at `version`, as the host's gzip tarball: every
    /// entry sits under one top-level directory the host names.
    ///
    /// Fails past [`MAX_ARCHIVE_BYTES`], and on a 404, which is how GitHub
    /// answers a private repository `token` cannot read.
    async fn fetch_archive(
        &self,
        remote: &RepositoryRemote,
        version: &str,
        token: Option<&str>,
    ) -> Result<Vec<u8>, SkillError>;
}

/// The largest repository archive [`SkillFetcher::fetch_archive`] reads, so a
/// plugin that vendors something huge fails instead of filling backend memory.
pub const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;

/// REST implementation of [`SkillFetcher`], over every host [`RepoApi`] reads.
pub struct GitHostSkillFetcher {
    client: Client,
}

impl GitHostSkillFetcher {
    /// A fetcher whose client is bounded by `timeouts`.
    ///
    /// Panics when the TLS backend cannot be loaded, as `reqwest::Client::new`
    /// does.
    #[expect(
        clippy::disallowed_methods,
        clippy::expect_used,
        reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
    )]
    pub fn new(timeouts: HttpTimeouts) -> Self {
        let client = Client::builder()
            .connect_timeout(timeouts.connect)
            .timeout(timeouts.total)
            .user_agent("moatless-vibe-backend")
            .build()
            .expect("the TLS backend initialises");
        Self { client }
    }
}

#[derive(Debug, Deserialize)]
struct ContentEntry {
    name: String,
    #[serde(rename = "type")]
    entry_type: String,
}

/// Contents API object for a single symlink path. Only the link target is
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
pub(crate) struct PluginMarketplace {
    #[serde(default)]
    pub(crate) plugins: Vec<PluginMarketplaceEntry>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PluginMarketplaceEntry {
    pub(crate) source: PluginMarketplaceSource,
}

/// One repository's REST surface, on whichever host it lives on.
///
/// GitHub and Forgejo answer the three reads a skill fetch needs — list a
/// directory, read a symlink's target, read a file — in the same JSON shapes,
/// under different roots, media types and authorization schemes. This type is
/// where that difference is stated; the layouts above it are one code path.
#[derive(Debug, Clone)]
struct RepoApi {
    host: SkillHost,
    /// What an endpoint path is appended to, e.g.
    /// `https://api.github.com/repos/{owner}/{name}`.
    root: String,
}

/// A git host whose contents API this module reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillHost {
    Github,
    Forgejo,
}

impl SkillHost {
    /// The host's name, for a message whose reader has to act on it.
    fn label(self) -> &'static str {
        match self {
            SkillHost::Github => "GitHub",
            SkillHost::Forgejo => "Forgejo",
        }
    }

    /// The scheme the host reads its own credential under. Forgejo takes a
    /// personal access token under `token`, which is how every other Forgejo
    /// call in this crate presents one.
    fn authorization(self, token: &str) -> String {
        match self {
            SkillHost::Github => format!("Bearer {token}"),
            SkillHost::Forgejo => format!("token {token}"),
        }
    }
}

impl RepoApi {
    /// The API `remote` is read through, or [`SkillError::UnsupportedProvider`]
    /// for a host with no contents surface wired here.
    fn of(remote: &RepositoryRemote) -> Result<Self, SkillError> {
        let unsupported = SkillError::UnsupportedProvider {
            git_url: remote.url.clone(),
        };
        let Some(repo) = remote.host_repo.as_ref() else {
            return Err(unsupported);
        };
        let (owner, name) = (repo.owner(), repo.name());
        match repo.provider() {
            RepositoryProvider::Github => Ok(Self {
                host: SkillHost::Github,
                root: format!("https://api.github.com/repos/{owner}/{name}"),
            }),
            RepositoryProvider::Forgejo => Ok(Self {
                host: SkillHost::Forgejo,
                root: crate::repository::remote::forgejo_repo_api(repo),
            }),
            RepositoryProvider::Gitlab | RepositoryProvider::Gitness => Err(unsupported),
        }
    }

    /// One directory's entries, or `None` when the directory is absent.
    async fn list_dir(
        &self,
        client: &Client,
        path: &str,
        token: Option<&str>,
        git_url: &str,
    ) -> Result<Option<Vec<ContentEntry>>, SkillError> {
        let accept = match self.host {
            SkillHost::Github => "application/vnd.github+json",
            SkillHost::Forgejo => "application/json",
        };
        self.get_json(client, &self.contents_url(path), accept, token, git_url)
            .await
    }

    /// What a symlinked entry points at — `None` when the entry is absent or
    /// names no target.
    async fn symlink_target(
        &self,
        client: &Client,
        path: &str,
        token: Option<&str>,
        git_url: &str,
    ) -> Result<Option<String>, SkillError> {
        // GitHub serves a symlink's own object only under this media type: ask
        // for the default one and it answers with the target file's content.
        let accept = match self.host {
            SkillHost::Github => "application/vnd.github.object+json",
            SkillHost::Forgejo => "application/json",
        };
        let link: Option<SymlinkObject> = self
            .get_json(client, &self.contents_url(path), accept, token, git_url)
            .await?;
        Ok(link.and_then(|link| link.target))
    }

    /// One file's contents as text, or `None` when it is absent.
    async fn read_file(
        &self,
        client: &Client,
        path: &str,
        token: Option<&str>,
        git_url: &str,
    ) -> Result<Option<String>, SkillError> {
        let (url, accept) = self.file_url(path);
        self.get_text(client, &url, accept, token, git_url).await
    }

    /// The default branch's head commit SHA, from one request returning 40
    /// bytes.
    ///
    /// GitHub only. `Ok(None)` is [`SkillFetcher::fetch_version`]'s "cannot be
    /// asked cheaply": Forgejo answers a page of commit JSON rather than a bare
    /// SHA, and plugin sync, the only caller, addresses GitHub.
    async fn head_sha(
        &self,
        client: &Client,
        token: Option<&str>,
        git_url: &str,
    ) -> Result<Option<String>, SkillError> {
        if self.host != SkillHost::Github {
            return Ok(None);
        }
        let sha = self
            .get_text(
                client,
                &format!("{}/commits/HEAD", self.root),
                "application/vnd.github.sha",
                token,
                git_url,
            )
            .await?;
        Ok(sha
            .map(|sha| sha.trim().to_string())
            .filter(|sha| !sha.is_empty()))
    }

    /// The repository at `version` as a gzip tarball, or `None` on a 404.
    async fn tarball(
        &self,
        client: &Client,
        version: &str,
        token: Option<&str>,
        git_url: &str,
    ) -> Result<Option<Vec<u8>>, SkillError> {
        let (url, accept) = match self.host {
            // Redirects to codeload, which reqwest follows without the
            // `Authorization` header; the redirect URL carries its own token.
            SkillHost::Github => (
                format!("{}/tarball/{version}", self.root),
                "application/vnd.github+json",
            ),
            SkillHost::Forgejo => (format!("{}/archive/{version}.tar.gz", self.root), "*/*"),
        };
        let Some(mut resp) = self.send(client, &url, accept, token, git_url).await? else {
            return Ok(None);
        };
        let fetch_error = |message: String| SkillError::Fetch {
            git_url: git_url.to_string(),
            message,
        };
        let mut data = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| fetch_error(format!("failed to read archive body: {e}")))?
        {
            if data.len() + chunk.len() > MAX_ARCHIVE_BYTES {
                return Err(fetch_error(format!(
                    "archive exceeds {MAX_ARCHIVE_BYTES} bytes"
                )));
            }
            data.extend_from_slice(&chunk);
        }
        Ok(Some(data))
    }

    fn contents_url(&self, path: &str) -> String {
        format!("{}/contents/{path}", self.root)
    }

    /// Where one file's own bytes are served, and the `Accept` that asks for
    /// them — the one endpoint the two hosts do not share.
    fn file_url(&self, path: &str) -> (String, &'static str) {
        match self.host {
            SkillHost::Github => (self.contents_url(path), "application/vnd.github.raw+json"),
            // Forgejo's contents object carries the file base64-encoded, so its
            // raw endpoint is the one that answers with the file itself.
            SkillHost::Forgejo => (format!("{}/raw/{path}", self.root), "*/*"),
        }
    }

    /// GET a contents URL and parse the body as JSON.
    ///
    /// Returns `Ok(None)` on 404, `Err(SkillError::AuthRequired)` on 401/403,
    /// `Err(SkillError::Fetch)` on any other non-success status or transport
    /// failure.
    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        client: &Client,
        url: &str,
        accept: &str,
        token: Option<&str>,
        git_url: &str,
    ) -> Result<Option<T>, SkillError> {
        let Some(resp) = self.send(client, url, accept, token, git_url).await? else {
            return Ok(None);
        };
        let host = self.host.label();
        let parsed = resp.json::<T>().await.map_err(|e| SkillError::Fetch {
            git_url: git_url.to_string(),
            message: format!("failed to decode {host} JSON response: {e}"),
        })?;
        Ok(Some(parsed))
    }

    /// GET a URL and return the body as text, or `None` on 404.
    async fn get_text(
        &self,
        client: &Client,
        url: &str,
        accept: &str,
        token: Option<&str>,
        git_url: &str,
    ) -> Result<Option<String>, SkillError> {
        let Some(resp) = self.send(client, url, accept, token, git_url).await? else {
            return Ok(None);
        };
        let host = self.host.label();
        let body = resp.text().await.map_err(|e| SkillError::Fetch {
            git_url: git_url.to_string(),
            message: format!("failed to read {host} response body: {e}"),
        })?;
        Ok(Some(body))
    }

    /// The one request both readers make, with every status they agree on
    /// already decided: `Ok(None)` is a 404.
    async fn send(
        &self,
        client: &Client,
        url: &str,
        accept: &str,
        token: Option<&str>,
        git_url: &str,
    ) -> Result<Option<reqwest::Response>, SkillError> {
        let host = self.host.label();
        let mut req = client
            .get(url)
            .header("User-Agent", moatless_core::USER_AGENT)
            .header("Accept", accept);
        if let Some(token) = token {
            req = req.header("Authorization", self.host.authorization(token));
        }

        let resp = req.send().await.map_err(|e| SkillError::Fetch {
            git_url: git_url.to_string(),
            message: format!("{host} API request failed: {e}"),
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
                message: format!("{host} API returned {status}: {body}"),
            });
        }
        Ok(Some(resp))
    }
}

#[async_trait]
impl SkillFetcher for GitHostSkillFetcher {
    async fn fetch_skill_files(
        &self,
        remote: &RepositoryRemote,
        layout: SkillLayout,
        token: Option<&str>,
    ) -> Result<Vec<FetchedSkillFile>, SkillError> {
        let remote_url = remote.url.as_str();
        let api = RepoApi::of(remote)?;

        match layout {
            SkillLayout::Workspace => {
                let mut groups: Vec<Vec<FetchedSkillFile>> = Vec::new();
                for prefix in REPO_SKILL_ROOTS {
                    match self
                        .fetch_skills_at_prefix(&api, prefix, token, remote_url, true)
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
                    resolve_plugin_skills_prefixes(&self.client, &api, token, remote_url).await?;
                let mut groups: Vec<Vec<FetchedSkillFile>> = Vec::new();
                for prefix in prefixes {
                    if let Some(files) = self
                        .fetch_skills_at_prefix(&api, &prefix, token, remote_url, false)
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

    /// See [`RepoApi::head_sha`] for which hosts answer this and which return
    /// `Ok(None)`; a remote with no contents surface at all returns `Ok(None)`
    /// too, leaving the fetch to fail where it can name the reason.
    async fn fetch_version(
        &self,
        remote: &RepositoryRemote,
        token: Option<&str>,
    ) -> Result<Option<String>, SkillError> {
        let Ok(api) = RepoApi::of(remote) else {
            return Ok(None);
        };
        api.head_sha(&self.client, token, remote.url.as_str()).await
    }

    async fn fetch_archive(
        &self,
        remote: &RepositoryRemote,
        version: &str,
        token: Option<&str>,
    ) -> Result<Vec<u8>, SkillError> {
        let git_url = remote.url.as_str();
        let api = RepoApi::of(remote)?;
        api.tarball(&self.client, version, token, git_url)
            .await?
            .ok_or_else(|| SkillError::Fetch {
                git_url: git_url.to_string(),
                message: format!(
                    "{} has no archive at {version}, or it cannot be read without a token",
                    api.host.label()
                ),
            })
    }
}

impl GitHostSkillFetcher {
    /// List one skills root (`skills_prefix`) and fetch every skill's
    /// `SKILL.md`. Returns `Ok(None)` when the root directory itself is absent
    /// (404), so the caller can distinguish "no such root" from "root present
    /// but empty".
    async fn fetch_skills_at_prefix(
        &self,
        api: &RepoApi,
        skills_prefix: &str,
        token: Option<&str>,
        remote_url: &str,
        skip_unreadable_entries: bool,
    ) -> Result<Option<Vec<FetchedSkillFile>>, SkillError> {
        let Some(entries) = api
            .list_dir(&self.client, skills_prefix, token, remote_url)
            .await?
        else {
            return Ok(None);
        };

        let fetches = entries
            .into_iter()
            .filter(|e| e.entry_type == "dir" || e.entry_type == "symlink")
            .map(|entry| {
                let api = api.clone();
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
                            &api,
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
                    let path = format!("{dir_path}/SKILL.md");
                    let content = match api
                        .read_file(&client, &path, token.as_deref(), &remote_url)
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
                    Ok::<Option<FetchedSkillFile>, SkillError>(content.map(|content| {
                        FetchedSkillFile {
                            dir_name,
                            path,
                            content,
                        }
                    }))
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

/// Resolve a symlinked skill entry to the repo-relative directory it points at.
///
/// Reads the symlink's `target` and resolves it against the symlink's
/// directory; returns `Ok(None)` (entry skipped) if the target is missing,
/// absolute, or escapes the repo root.
async fn resolve_symlink_dir(
    client: &Client,
    api: &RepoApi,
    skills_prefix: &str,
    name: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<Option<String>, SkillError> {
    let Some(target) = api
        .symlink_target(client, &format!("{skills_prefix}/{name}"), token, git_url)
        .await?
    else {
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
/// Supports single-plugin and marketplace repos; local marketplace entries
/// resolve relative to the repository root, matching both Codex's and Claude
/// Code's marketplace.json conventions.
async fn resolve_plugin_skills_prefixes(
    client: &Client,
    api: &RepoApi,
    token: Option<&str>,
    git_url: &str,
) -> Result<Vec<String>, SkillError> {
    if let Some(prefix) =
        resolve_plugin_manifest_skills_prefix(client, api, "", token, git_url).await?
    {
        return Ok(vec![prefix]);
    }

    let Some((marketplace_path, raw)) =
        fetch_first_marketplace(client, api, token, git_url).await?
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
        let prefix =
            resolve_plugin_manifest_skills_prefix(client, api, &plugin_root, token, git_url)
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
    api: &RepoApi,
    token: Option<&str>,
    git_url: &str,
) -> Result<Option<(&'static str, String)>, SkillError> {
    for marketplace_path in PLUGIN_MARKETPLACE_PATHS {
        if let Some(raw) = api
            .read_file(client, marketplace_path, token, git_url)
            .await?
        {
            return Ok(Some((marketplace_path, raw)));
        }
    }
    Ok(None)
}

async fn resolve_plugin_manifest_skills_prefix(
    client: &Client,
    api: &RepoApi,
    plugin_root: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<Option<String>, SkillError> {
    for manifest_path in PLUGIN_MANIFEST_PATHS {
        let repo_path = join_repo_paths(plugin_root, manifest_path);
        let Some(raw) = api.read_file(client, &repo_path, token, git_url).await? else {
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

/// Normalize a `plugin.json` `skills` value into a repo-relative contents API
/// path segment — strips a leading `./` and trailing `/`, rejects absolute
/// paths or any `..` segment.
pub(crate) fn normalize_skills_path(raw: &str) -> Result<String, String> {
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
            _remote: &RepositoryRemote,
            _layout: SkillLayout,
            _token: Option<&str>,
        ) -> Result<Vec<FetchedSkillFile>, SkillError> {
            Ok(self.files.clone())
        }

        async fn fetch_version(
            &self,
            _remote: &RepositoryRemote,
            _token: Option<&str>,
        ) -> Result<Option<String>, SkillError> {
            Ok(None)
        }

        async fn fetch_archive(
            &self,
            _remote: &RepositoryRemote,
            _version: &str,
            _token: Option<&str>,
        ) -> Result<Vec<u8>, SkillError> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn stub_returns_injected_files() {
        let stub = StubFetcher {
            files: vec![FetchedSkillFile {
                dir_name: "alpha".into(),
                path: ".claude/skills/alpha/SKILL.md".into(),
                content: "---\nname: alpha\ndescription: A\n---\n".into(),
            }],
        };
        let files = stub
            .fetch_skill_files(
                &crate::repository::remote::resolve_remote(
                    Some(RepositoryProvider::Github),
                    "https://github.com/x/y.git".to_string(),
                ),
                SkillLayout::Workspace,
                None,
            )
            .await
            .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].dir_name, "alpha");
    }

    #[test]
    fn dedup_by_dir_name_merges_claude_and_agents_prefixes() {
        let file = |dir: &str, body: &str| FetchedSkillFile {
            dir_name: dir.into(),
            path: format!(".claude/skills/{dir}/SKILL.md"),
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
    async fn a_host_with_no_contents_surface_is_unsupported() {
        let fetcher =
            GitHostSkillFetcher::new(moatless_core::config::HttpConfig::default().git_content);
        let err = fetcher
            .fetch_skill_files(
                &crate::repository::remote::resolve_remote(
                    Some(RepositoryProvider::Gitlab),
                    "https://gitlab.com/foo/bar.git".to_string(),
                ),
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

    fn api_of(provider: RepositoryProvider, url: &str) -> RepoApi {
        RepoApi::of(&crate::repository::remote::resolve_remote(
            Some(provider),
            url.to_string(),
        ))
        .expect("a supported host")
    }

    #[test]
    fn a_forgejo_repository_is_addressed_on_the_host_its_remote_names() {
        // The instance is whatever the remote URL carries — "Forgejo" names no
        // single host the way `api.github.com` does.
        let api = api_of(
            RepositoryProvider::Forgejo,
            "https://git.example.com/juni/config-juni-dev.git",
        );

        assert_eq!(api.host, SkillHost::Forgejo);
        assert_eq!(
            api.contents_url(".claude/skills"),
            "https://git.example.com/api/v1/repos/juni/config-juni-dev/contents/.claude/skills"
        );
        // The one endpoint that is not GitHub's: a typo here 404s, and
        // `should_skip_skill_entry_fetch_error` swallows that into no skills.
        assert_eq!(
            api.file_url(".claude/skills/deploy/SKILL.md").0,
            "https://git.example.com/api/v1/repos/juni/config-juni-dev/raw/.claude/skills/deploy/SKILL.md"
        );
        assert_eq!(api.host.authorization("fj-token"), "token fj-token");
    }

    #[test]
    fn a_forgejo_remote_on_http_is_still_read_over_https() {
        let api = api_of(
            RepositoryProvider::Forgejo,
            "http://git.example.com/juni/config-juni-dev",
        );

        assert!(api.root.starts_with("https://git.example.com/api/v1/"));
    }

    #[test]
    fn a_github_repository_keeps_its_own_api_root_and_bearer_token() {
        let api = api_of(RepositoryProvider::Github, "https://github.com/x/y.git");

        assert_eq!(api.host, SkillHost::Github);
        assert_eq!(
            api.contents_url(".agents/skills"),
            "https://api.github.com/repos/x/y/contents/.agents/skills"
        );
        assert_eq!(
            api.file_url(".agents/skills/deploy/SKILL.md").0,
            "https://api.github.com/repos/x/y/contents/.agents/skills/deploy/SKILL.md"
        );
        assert_eq!(api.host.authorization("gh-token"), "Bearer gh-token");
    }

    #[tokio::test]
    async fn only_github_answers_a_cheap_version_probe() {
        let fetcher =
            GitHostSkillFetcher::new(moatless_core::config::HttpConfig::default().git_content);
        let remote = crate::repository::remote::resolve_remote(
            Some(RepositoryProvider::Forgejo),
            "https://git.example.com/juni/config-juni-dev.git".to_string(),
        );

        // No request is made, so this passes without a reachable host: `None`
        // tells plugin sync to fetch rather than trust a stored version.
        assert_eq!(fetcher.fetch_version(&remote, None).await.unwrap(), None);
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
