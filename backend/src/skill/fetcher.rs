//! GitHub REST API-based fetcher for skill directories.
//!
//! Supports two layouts:
//!
//! - [`SkillLayout::Workspace`] — skills live at `{name}/SKILL.md` under either
//!   `.claude/skills` (Claude) or `.agents/skills` (Codex). Both roots are
//!   listed and merged; a skill exposed in both (typically `.claude/skills/foo`
//!   symlinking to `../../.agents/skills/foo`) is de-duplicated by directory
//!   name, with `.claude/skills` winning.
//! - [`SkillLayout::Plugin`] — Claude Code plugin repo. Probes
//!   `.claude-plugin/plugin.json` to resolve the skills directory, defaulting
//!   to `skills/` at the repo root per the plugin spec.
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
    /// Claude Code plugin repo: resolve the skills directory from
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

/// Subset of the Claude Code plugin manifest we need: only the optional
/// `skills` path. All other fields are ignored.
#[derive(Debug, Deserialize)]
struct PluginManifest {
    #[serde(default)]
    skills: Option<String>,
}

/// Workspace skill roots, in precedence order. Both are listed and merged so a
/// repo that stores its real skills under `.agents/skills` (Codex) is picked up
/// whether or not it also exposes them via `.claude/skills` (Claude) symlinks.
const WORKSPACE_SKILL_PREFIXES: [&str; 2] = [".claude/skills", ".agents/skills"];

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
                    if let Some(files) = self
                        .fetch_skills_at_prefix(&owner, &repo, prefix, token, remote_url)
                        .await?
                    {
                        groups.push(files);
                    } else {
                        debug!(remote_url, prefix, "No skills directory on remote");
                    }
                }
                Ok(dedup_by_dir_name(groups))
            }
            SkillLayout::Plugin => {
                let prefix =
                    resolve_plugin_skills_prefix(&self.client, &owner, &repo, token, remote_url)
                        .await?;
                if let Some(files) = self
                    .fetch_skills_at_prefix(&owner, &repo, &prefix, token, remote_url)
                    .await?
                {
                    Ok(files)
                } else {
                    warn!(
                        git_url = %remote_url,
                        skills_prefix = %prefix,
                        "Plugin repo has no skills directory — skipping"
                    );
                    Ok(Vec::new())
                }
            }
        }
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
                    let dir_name = entry.name;
                    let dir_path = if entry.entry_type == "symlink" {
                        match resolve_symlink_dir(
                            &client,
                            &owner,
                            &repo,
                            &skills_prefix,
                            &dir_name,
                            token.as_deref(),
                            &remote_url,
                        )
                        .await?
                        {
                            Some(path) => path,
                            None => return Ok(None),
                        }
                    } else {
                        format!("{skills_prefix}/{dir_name}")
                    };
                    let file_url = format!(
                        "https://api.github.com/repos/{owner}/{repo}/contents/{dir_path}/SKILL.md"
                    );
                    let content =
                        fetch_github_raw(&client, &file_url, token.as_deref(), &remote_url).await?;
                    Ok::<Option<FetchedSkillFile>, SkillError>(
                        content.map(|content| FetchedSkillFile { dir_name, content }),
                    )
                }
            });

        let results: Vec<Option<FetchedSkillFile>> = try_join_all(fetches).await?;
        Ok(Some(results.into_iter().flatten().collect()))
    }
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

/// Probe `.claude-plugin/plugin.json` and return the repo-relative path at
/// which the plugin stores its skills. Defaults to `skills` when the
/// manifest is absent or does not override the field.
async fn resolve_plugin_skills_prefix(
    client: &Client,
    owner: &str,
    repo: &str,
    token: Option<&str>,
    git_url: &str,
) -> Result<String, SkillError> {
    let manifest_url =
        format!("https://api.github.com/repos/{owner}/{repo}/contents/.claude-plugin/plugin.json");
    let Some(raw) = fetch_github_raw(client, &manifest_url, token, git_url).await? else {
        return Ok("skills".to_string());
    };
    let manifest: PluginManifest = serde_json::from_str(&raw).map_err(|e| SkillError::Fetch {
        git_url: git_url.to_string(),
        message: format!("invalid .claude-plugin/plugin.json: {e}"),
    })?;
    let raw_path = manifest.skills.as_deref().unwrap_or("skills");
    normalize_skills_path(raw_path).map_err(|message| SkillError::Fetch {
        git_url: git_url.to_string(),
        message,
    })
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
    let mut req = client
        .get(url)
        .header("User-Agent", "moatless-vibe-backend")
        .header("Accept", "application/vnd.github.raw+json");
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
    fn normalize_skills_path_strips_prefix_and_trailing_slash() {
        assert_eq!(
            normalize_skills_path("./custom/skills/").unwrap(),
            "custom/skills"
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
}
