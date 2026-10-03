//! Point the gh CLI at [`super::github_proxy`]'s socket.
//!
//! `config.yml` gains `http_unix_socket`, which sends every `gh` request over
//! the socket. `hosts.yml` holds [`PLACEHOLDER_TOKEN`] rather than a token:
//! `gh` refuses to run until it finds one, and the socket drops whatever `gh`
//! presents before a request leaves the pod. This module is the only writer of
//! `hosts.yml`, and renders the whole file rather than merging.

use std::path::{Path, PathBuf};

/// What `gh` presents as its token. Never a credential; see the module doc.
pub const PLACEHOLDER_TOKEN: &str = "moatless-github-proxy";

/// The login recorded beside the placeholder. `gh`'s multi-account format
/// requires some user key; this one is what git uses for an installation token.
const LOGIN: &str = "x-access-token";

/// The `config.yml` key that routes `gh`'s HTTP over a unix socket.
const UNIX_SOCKET_KEY: &str = "http_unix_socket";

/// Route `gh` under `home_dir` through `socket`. Idempotent, and keeps every
/// other `config.yml` setting.
pub async fn route_gh_through(home_dir: &str, socket: &Path) -> Result<(), String> {
    let dir = gh_dir(home_dir);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;

    let config_path = dir.join("config.yml");
    let config = match tokio::fs::read_to_string(&config_path).await {
        Ok(config) => config,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("Failed to read {}: {e}", config_path.display())),
    };
    write_atomically(&config_path, &with_unix_socket(&config, socket)).await?;
    write_atomically(&dir.join("hosts.yml"), &render_hosts_yml()).await
}

fn gh_dir(home_dir: &str) -> PathBuf {
    PathBuf::from(home_dir).join(".config").join("gh")
}

/// `config` with its top-level [`UNIX_SOCKET_KEY`] set to `socket`.
fn with_unix_socket(config: &str, socket: &Path) -> String {
    let prefix = format!("{UNIX_SOCKET_KEY}:");
    let mut lines: Vec<String> = config
        .lines()
        .filter(|line| !line.starts_with(&prefix))
        .map(str::to_string)
        .collect();
    // A JSON string is a YAML double-quoted scalar, so any path survives.
    let quoted = serde_json::to_string(&socket.display().to_string())
        .unwrap_or_else(|_| socket.display().to_string());
    lines.push(format!("{prefix} {quoted}"));
    lines.push(String::new());
    lines.join("\n")
}

/// Render the single-host `hosts.yml` `gh` expects.
///
/// Writes both the top-level `oauth_token` (older `gh`) and the `users` entry
/// (multi-account migration), so neither version has to migrate the file.
fn render_hosts_yml() -> String {
    format!(
        "github.com:\n    \
         users:\n        \
         {LOGIN}:\n            \
         oauth_token: {PLACEHOLDER_TOKEN}\n    \
         oauth_token: {PLACEHOLDER_TOKEN}\n    \
         user: {LOGIN}\n    \
         git_protocol: https\n"
    )
}

/// Write `contents` to `path` via a same-directory rename, so a racing `gh`
/// never reads a partial file.
async fn write_atomically(path: &Path, contents: &str) -> Result<(), String> {
    let tmp = path.with_extension("yml.tmp");
    if let Err(e) = tokio::fs::write(&tmp, contents).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(format!("Failed to write {}: {e}", tmp.display()));
    }
    tokio::fs::rename(&tmp, path).await.map_err(|e| {
        format!(
            "Failed to move {} into place at {}: {e}",
            tmp.display(),
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOCKET: &str = "/run/moatless/github.sock";

    async fn read(home: &str, file: &str) -> String {
        tokio::fs::read_to_string(gh_dir(home).join(file))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_fresh_home_is_routed_through_the_socket_with_only_a_placeholder() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path().join("fresh-home").display().to_string();

        route_gh_through(&home, Path::new(SOCKET)).await.unwrap();

        assert_eq!(
            read(&home, "config.yml").await,
            format!("http_unix_socket: \"{SOCKET}\"\n")
        );
        let hosts = read(&home, "hosts.yml").await;
        // Legacy readers take the top-level key, multi-account ones the nested
        // user entry. Missing either forces gh to migrate the file on first use.
        assert!(hosts.contains(&format!("\n    oauth_token: {PLACEHOLDER_TOKEN}\n")));
        assert!(hosts.contains(&format!("\n            oauth_token: {PLACEHOLDER_TOKEN}\n")));
        assert!(hosts.contains(&format!("\n    user: {LOGIN}\n")));
        assert!(!gh_dir(&home).join("hosts.yml.tmp").exists());
    }

    /// A token a previous image wrote must not survive the first start of
    /// this one.
    #[tokio::test]
    async fn a_token_left_in_hosts_yml_is_replaced() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path().display().to_string();
        tokio::fs::create_dir_all(gh_dir(&home)).await.unwrap();
        tokio::fs::write(
            gh_dir(&home).join("hosts.yml"),
            "github.com:\n    oauth_token: ghs_stale\n",
        )
        .await
        .unwrap();

        route_gh_through(&home, Path::new(SOCKET)).await.unwrap();

        assert!(!read(&home, "hosts.yml").await.contains("ghs_stale"));
    }

    #[tokio::test]
    async fn other_settings_survive_and_the_socket_is_set_once() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path().display().to_string();
        tokio::fs::create_dir_all(gh_dir(&home)).await.unwrap();
        tokio::fs::write(
            gh_dir(&home).join("config.yml"),
            "git_protocol: https\naliases:\n    co: pr checkout\nhttp_unix_socket: /old.sock\n",
        )
        .await
        .unwrap();

        route_gh_through(&home, Path::new(SOCKET)).await.unwrap();
        route_gh_through(&home, Path::new(SOCKET)).await.unwrap();

        assert_eq!(
            read(&home, "config.yml").await,
            format!(
                "git_protocol: https\naliases:\n    co: pr checkout\nhttp_unix_socket: \"{SOCKET}\"\n"
            )
        );
    }
}
