//! Live control channel for preview-server sidecars.
//!
//! Preview servers run as sidecar containers whose start command and env are
//! baked into the pod/container spec, so changing either normally means
//! rebuilding the pod — which restarts the agent container too. This module is
//! the other half of the escape hatch: the sandbox writes an override into the
//! workspace volume that every sidecar mounts, and the sidecar's supervisor
//! wrapper re-reads it whenever the generation counter changes.
//!
//! The files live on `WORKSPACE_DIR` rather than the shared data volume because
//! that is the only volume with any durability: `/opt/moatless/data` is an
//! `emptyDir` in K8s, wiped whenever the pod is deleted, which a stop/start, a
//! redeploy, and the reaper's idle-stop all do. Under `WORKSPACE_DIR` the
//! override rides the same storage as the repos — the EBS PVC on the default
//! tier, the whole-workspace tar on the `s3` tier.
//!
//! They sit in a dot-directory at the workspace root, a sibling of the repo
//! checkouts rather than inside one, so they are never mistaken for repo content.
//!
//! Layout under `<workspace_dir>/.moatless/servers/<name>/`:
//!
//! | File | Meaning |
//! |---|---|
//! | `command` | start-command override; absent = use the baked command |
//! | `env` | shell fragment sourced after `server.env`; absent = no override |
//! | `generation` | monotonic counter — a bump means "restart the child" |
//!
//! The override is whole-state, not a delta: the backend owns the persisted
//! copy and always sends the complete set, so this module never merges.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tokio::fs;
use tracing::{info, warn};

use crate::init::server_env::shell_single_quote;

/// A server's complete override state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerOverride {
    /// Replaces the sidecar's baked start command. `None` = use the baked one.
    #[serde(default)]
    pub start_command: Option<String>,
    /// Env entries exported after `server.env`, so they win over it.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Keys emitted as `unset K` — the only way to drop a variable the
    /// container spec injected, since sourcing can add but never remove.
    #[serde(default)]
    pub unset_env: Vec<String>,
}

/// Reject names that would escape the control directory. Server names come from
/// workspace config and reach us over gRPC, so they are untrusted input that is
/// about to become a path component.
pub fn is_valid_server_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Control-channel root, relative to `WORKSPACE_DIR`. Dot-prefixed and outside
/// any repo checkout.
const CONTROL_SUBDIR: &str = ".moatless/servers";

fn control_dir(workspace_dir: &str, server_name: &str) -> PathBuf {
    Path::new(workspace_dir)
        .join(CONTROL_SUBDIR)
        .join(server_name)
}

/// Write `server_name`'s override and, when `restart`, bump its generation so
/// the sidecar re-execs its child. Returns the generation after the call.
///
/// Callers that only want a restart pass the unchanged override — writing it
/// again is idempotent, and the generation bump is what the sidecar reacts to.
pub async fn apply(
    workspace_dir: &str,
    server_name: &str,
    ov: &ServerOverride,
    restart: bool,
) -> std::io::Result<u64> {
    if !is_valid_server_name(server_name) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid server name: {server_name}"),
        ));
    }

    let dir = control_dir(workspace_dir, server_name);
    fs::create_dir_all(&dir).await?;

    write_or_remove(&dir.join("command"), ov.start_command.as_deref()).await?;
    let env_file = if ov.env.is_empty() && ov.unset_env.is_empty() {
        None
    } else {
        Some(render_env_fragment(&ov.env, &ov.unset_env))
    };
    write_or_remove(&dir.join("env"), env_file.as_deref()).await?;

    let generation = if restart {
        bump_generation(&dir).await?
    } else {
        read_generation(&dir).await
    };

    info!(
        server = server_name,
        generation,
        restart,
        has_command = ov.start_command.is_some(),
        env_count = ov.env.len(),
        unset_count = ov.unset_env.len(),
        "Applied server config override"
    );
    Ok(generation)
}

/// Overrides as delivered in `REPO_CONFIG.serverOverrides`, keyed by server name.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoConfigOverrides {
    #[serde(default)]
    server_overrides: BTreeMap<String, ServerOverride>,
}

/// Parse the persisted overrides the backend baked into `REPO_CONFIG`.
///
/// The DB stays the record even though the control files now live on a durable
/// volume, because that volume is only durable on some tiers: `ephemeral` keeps
/// nothing, and `node` keeps it only while the replacement pod lands on the same
/// node. Malformed JSON degrades to "no overrides" rather than blocking init.
pub fn parse_overrides(repo_config_raw: Option<&str>) -> BTreeMap<String, ServerOverride> {
    let Some(raw) = repo_config_raw.filter(|r| !r.is_empty()) else {
        return BTreeMap::new();
    };
    match serde_json::from_str::<RepoConfigOverrides>(raw) {
        Ok(parsed) => parsed.server_overrides,
        Err(e) => {
            warn!(error = %e, "Failed to parse serverOverrides from REPO_CONFIG");
            BTreeMap::new()
        }
    }
}

/// Materialize overrides for every server at init, before the sidecars start.
///
/// Servers absent from `overrides` get their control files cleared, so a
/// removed override does not survive on the workspace volume across a restart.
pub async fn materialize(
    workspace_dir: &str,
    overrides: &BTreeMap<String, ServerOverride>,
    known_servers: &[String],
) {
    for name in known_servers {
        let ov = overrides.get(name).cloned().unwrap_or_default();
        if let Err(e) = apply(workspace_dir, name, &ov, false).await {
            warn!(server = name, error = %e, "Failed to materialize server override");
        }
    }
}

/// Write `contents`, or remove the file when `None` — absent means "no
/// override", which is what the sidecar falls back on.
async fn write_or_remove(path: &Path, contents: Option<&str>) -> std::io::Result<()> {
    match contents {
        Some(contents) => write_atomic(path, contents).await,
        None => match fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        },
    }
}

/// Write via tmp + rename so a sidecar starting concurrently never reads a
/// half-written command or env fragment.
async fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents).await?;
    set_owner_only_perms(&tmp).await;
    fs::rename(&tmp, path).await
}

/// Override values are configuration rather than secrets, but they can carry a
/// token a user pasted in, so they get the same `0600` treatment as
/// `server.env`. Worth more here than it was on the data volume: this file now
/// rides the workspace, which the `s3` tier tars and uploads.
async fn set_owner_only_perms(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await {
            warn!(path = %path.display(), error = %e, "Failed to chmod server control file");
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

async fn read_generation(dir: &Path) -> u64 {
    fs::read_to_string(dir.join("generation"))
        .await
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

async fn bump_generation(dir: &Path) -> std::io::Result<u64> {
    let next = read_generation(dir).await.saturating_add(1);
    write_atomic(&dir.join("generation"), &format!("{next}\n")).await?;
    Ok(next)
}

fn render_env_fragment(env: &BTreeMap<String, String>, unset: &[String]) -> String {
    let mut out = String::from("# Generated by moatless. Do not commit.\n");
    for (k, v) in env {
        out.push_str("export ");
        out.push_str(k);
        out.push('=');
        out.push_str(&shell_single_quote(v));
        out.push('\n');
    }
    // Unsets last so an explicit unset of a key also present in `env` wins —
    // the backend rejects that overlap, this is belt and braces.
    for k in unset {
        out.push_str("unset ");
        out.push_str(k);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn workspace_dir(tmp: &TempDir) -> String {
        tmp.path().display().to_string()
    }

    #[test]
    fn rejects_names_that_escape_the_control_dir() {
        assert!(is_valid_server_name("dev"));
        assert!(is_valid_server_name("api-server_2"));
        assert!(!is_valid_server_name(""));
        assert!(!is_valid_server_name("../etc"));
        assert!(!is_valid_server_name("a/b"));
        assert!(!is_valid_server_name("."));
    }

    /// The path is a contract with the supervisor script the backend generates
    /// (`backend/src/sandbox/server_runner.rs`), which hardcodes the absolute
    /// form. Moving it on one side only would silently disable every override.
    #[tokio::test]
    async fn control_files_live_under_the_workspace_not_the_data_volume() {
        let tmp = TempDir::new().unwrap();
        let ov = ServerOverride {
            start_command: Some("bun run dev".to_string()),
            ..Default::default()
        };
        apply(&workspace_dir(&tmp), "dev", &ov, true).await.unwrap();

        let dir = tmp.path().join(".moatless").join("servers").join("dev");
        assert!(dir.join("command").exists());
        assert!(dir.join("generation").exists());
        // Nothing lands at the old shared-data location.
        assert!(!tmp.path().join("servers").exists());
    }

    #[tokio::test]
    async fn apply_rejects_invalid_server_name() {
        let tmp = TempDir::new().unwrap();
        let err = apply(&workspace_dir(&tmp), "../escape", &ServerOverride::default(), false)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn empty_override_writes_no_control_files() {
        let tmp = TempDir::new().unwrap();
        apply(&workspace_dir(&tmp), "dev", &ServerOverride::default(), false)
            .await
            .unwrap();

        let dir = tmp.path().join(CONTROL_SUBDIR).join("dev");
        assert!(!dir.join("command").exists());
        assert!(!dir.join("env").exists());
    }

    #[tokio::test]
    async fn writes_command_and_env_fragment() {
        let tmp = TempDir::new().unwrap();
        let ov = ServerOverride {
            start_command: Some("bun run dev".to_string()),
            env: BTreeMap::from([
                ("ZULU".to_string(), "last".to_string()),
                ("QUOTE".to_string(), "a'b".to_string()),
            ]),
            unset_env: vec!["LOG_LEVEL".to_string()],
        };
        apply(&workspace_dir(&tmp), "dev", &ov, false).await.unwrap();

        let dir = tmp.path().join(CONTROL_SUBDIR).join("dev");
        assert_eq!(
            fs::read_to_string(dir.join("command")).await.unwrap(),
            "bun run dev"
        );
        let env = fs::read_to_string(dir.join("env")).await.unwrap();
        let body: Vec<&str> = env.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            body,
            vec!["export QUOTE='a'\\''b'", "export ZULU='last'", "unset LOG_LEVEL"]
        );
    }

    #[tokio::test]
    async fn clearing_an_override_removes_the_files() {
        let tmp = TempDir::new().unwrap();
        let ov = ServerOverride {
            start_command: Some("sleep 1".to_string()),
            env: BTreeMap::from([("A".to_string(), "1".to_string())]),
            unset_env: vec![],
        };
        apply(&workspace_dir(&tmp), "dev", &ov, false).await.unwrap();
        apply(&workspace_dir(&tmp), "dev", &ServerOverride::default(), false)
            .await
            .unwrap();

        let dir = tmp.path().join(CONTROL_SUBDIR).join("dev");
        assert!(!dir.join("command").exists());
        assert!(!dir.join("env").exists());
    }

    #[tokio::test]
    async fn generation_only_advances_when_restart_requested() {
        let tmp = TempDir::new().unwrap();
        let dir = workspace_dir(&tmp);
        let ov = ServerOverride::default();

        assert_eq!(apply(&dir, "dev", &ov, false).await.unwrap(), 0);
        assert_eq!(apply(&dir, "dev", &ov, true).await.unwrap(), 1);
        assert_eq!(apply(&dir, "dev", &ov, false).await.unwrap(), 1);
        assert_eq!(apply(&dir, "dev", &ov, true).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn materialize_clears_servers_without_an_override() {
        let tmp = TempDir::new().unwrap();
        let dir = workspace_dir(&tmp);
        let ov = ServerOverride {
            start_command: Some("stale".to_string()),
            ..Default::default()
        };
        apply(&dir, "dev", &ov, false).await.unwrap();

        materialize(
            &dir,
            &BTreeMap::new(),
            &["dev".to_string(), "api".to_string()],
        )
        .await;

        assert!(!tmp.path().join(CONTROL_SUBDIR).join("dev/command").exists());
        assert!(tmp.path().join(CONTROL_SUBDIR).join("api").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn control_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let ov = ServerOverride {
            env: BTreeMap::from([("TOKEN".to_string(), "shh".to_string())]),
            ..Default::default()
        };
        apply(&workspace_dir(&tmp), "dev", &ov, true).await.unwrap();

        let meta = fs::metadata(tmp.path().join(CONTROL_SUBDIR).join("dev/env")).await.unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
}
