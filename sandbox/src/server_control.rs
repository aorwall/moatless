//! Live control channel for preview-server sidecars.
//!
//! Preview servers run as sidecar containers whose start command and env are
//! baked into the pod/container spec, so changing either normally means
//! rebuilding the pod — which restarts the agent container too. This module is
//! the other half of the escape hatch: the sandbox writes an override into the
//! workspace volume that every sidecar mounts, and the sidecar's supervisor
//! wrapper re-reads it whenever the generation counter changes.
//!
//! **These files are the record.** There is no database copy to replay from:
//! an override is workspace state, and it lives exactly where the rest of the
//! workspace lives. On the `ebs` tier that is the PVC; on `s3` it is the
//! whole-workspace tar; in Docker it is the host bind mount. On `ephemeral` it
//! is gone when the pod is — which is that tier's contract, the repo checkout
//! goes with it.
//!
//! They sit in a dot-directory at the workspace root, a sibling of the repo
//! checkouts rather than inside one, so they are never mistaken for repo content.
//!
//! Layout under `<workspace_dir>/.moatless/servers/<name>/`:
//!
//! | File | Meaning | Read by |
//! |---|---|---|
//! | `override.json` | the authored override, verbatim | this module |
//! | `command` | start-command override; absent = use the baked command | sidecar |
//! | `env` | shell fragment sourced after `server.env`; absent = no override | sidecar |
//! | `generation` | monotonic counter — a bump means "restart the child" | sidecar + this module |
//!
//! `command` and `env` are *derived* from `override.json` for the sidecar's
//! benefit: its supervisor is bash in an arbitrary `docker_image` and cannot be
//! assumed to have a JSON parser. `override.json` is what [`read_all`] reads
//! back, because re-parsing the rendered shell fragment would not round-trip.
//!
//! The override is whole-state, not a delta: callers always send the complete
//! set, so this module never merges.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::fs;
use tracing::{info, warn};

use crate::init::server_env::shell_single_quote;

/// A server's complete override state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

impl ServerOverride {
    /// No override at all — the sidecar runs entirely off its container spec.
    /// Applying an empty override is how `reset` works.
    pub fn is_empty(&self) -> bool {
        self.start_command.is_none() && self.env.is_empty() && self.unset_env.is_empty()
    }
}

/// What a server's control directory currently holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveOverride {
    pub config: ServerOverride,
    /// The counter the sidecar is watching; `0` means it has never restarted.
    pub generation: u64,
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

    // The authored form first, then the two files derived from it. Writing the
    // record before the derived copies means a crash in between leaves the
    // sidecar on its old (still-consistent) config rather than on a state
    // nothing can read back.
    let authored = if ov.is_empty() {
        None
    } else {
        Some(serde_json::to_string_pretty(ov)?)
    };
    write_or_remove(&dir.join("override.json"), authored.as_deref()).await?;

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

/// Read back every override currently on disk, keyed by server name.
///
/// Servers with no override are absent from the map rather than present with an
/// empty value, so the caller can tell "never set" from "set to nothing".
/// A directory that is unreadable or holds malformed JSON is skipped with a
/// warning: this backs a status read, and a corrupt file for one server must not
/// blank out the others.
pub async fn read_all(workspace_dir: &str) -> BTreeMap<String, LiveOverride> {
    let root = Path::new(workspace_dir).join(CONTROL_SUBDIR);
    let mut out = BTreeMap::new();

    let mut entries = match fs::read_dir(&root).await {
        Ok(entries) => entries,
        // No control root means no override has ever been applied.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return out,
        Err(e) => {
            warn!(path = %root.display(), error = %e, "Failed to list server control dir");
            return out;
        }
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_valid_server_name(&name) {
            continue;
        }
        let dir = entry.path();
        let raw = match fs::read_to_string(dir.join("override.json")).await {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                warn!(server = name, error = %e, "Failed to read server override");
                continue;
            }
        };
        match serde_json::from_str::<ServerOverride>(&raw) {
            Ok(config) => {
                let generation = read_generation(&dir).await;
                out.insert(name, LiveOverride { config, generation });
            }
            Err(e) => warn!(server = name, error = %e, "Malformed server override on disk"),
        }
    }
    out
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
        let err = apply(
            &workspace_dir(&tmp),
            "../escape",
            &ServerOverride::default(),
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn empty_override_writes_no_control_files() {
        let tmp = TempDir::new().unwrap();
        apply(
            &workspace_dir(&tmp),
            "dev",
            &ServerOverride::default(),
            false,
        )
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
        apply(&workspace_dir(&tmp), "dev", &ov, false)
            .await
            .unwrap();

        let dir = tmp.path().join(CONTROL_SUBDIR).join("dev");
        assert_eq!(
            fs::read_to_string(dir.join("command")).await.unwrap(),
            "bun run dev"
        );
        let env = fs::read_to_string(dir.join("env")).await.unwrap();
        let body: Vec<&str> = env.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            body,
            vec![
                "export QUOTE='a'\\''b'",
                "export ZULU='last'",
                "unset LOG_LEVEL"
            ]
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
        apply(&workspace_dir(&tmp), "dev", &ov, false)
            .await
            .unwrap();
        apply(
            &workspace_dir(&tmp),
            "dev",
            &ServerOverride::default(),
            false,
        )
        .await
        .unwrap();

        let dir = tmp.path().join(CONTROL_SUBDIR).join("dev");
        assert!(!dir.join("command").exists());
        assert!(!dir.join("env").exists());
        assert!(!dir.join("override.json").exists());
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

    /// The disk is the record now, so a written override has to survive a
    /// round-trip exactly — this is what `servers list` reports.
    #[tokio::test]
    async fn read_all_round_trips_what_apply_wrote() {
        let tmp = TempDir::new().unwrap();
        let dir = workspace_dir(&tmp);
        let ov = ServerOverride {
            start_command: Some("bun run dev".to_string()),
            env: BTreeMap::from([("A".to_string(), "a'b".to_string())]),
            unset_env: vec!["LOG_LEVEL".to_string()],
        };
        apply(&dir, "dev", &ov, true).await.unwrap();

        let all = read_all(&dir).await;
        assert_eq!(all.len(), 1);
        assert_eq!(all["dev"].config, ov);
        assert_eq!(all["dev"].generation, 1);
    }

    /// A server that was reset must read back as absent, not as an empty
    /// override — otherwise a reset looks like "override set to nothing".
    #[tokio::test]
    async fn read_all_omits_servers_whose_override_was_cleared() {
        let tmp = TempDir::new().unwrap();
        let dir = workspace_dir(&tmp);
        let ov = ServerOverride {
            start_command: Some("stale".to_string()),
            ..Default::default()
        };
        apply(&dir, "dev", &ov, false).await.unwrap();
        apply(&dir, "api", &ov, false).await.unwrap();

        apply(&dir, "dev", &ServerOverride::default(), true)
            .await
            .unwrap();

        let all = read_all(&dir).await;
        assert_eq!(all.keys().collect::<Vec<_>>(), vec!["api"]);
    }

    #[tokio::test]
    async fn read_all_on_a_workspace_with_no_overrides_is_empty() {
        let tmp = TempDir::new().unwrap();
        assert!(read_all(&workspace_dir(&tmp)).await.is_empty());
    }

    /// One corrupt file must not blank out the other servers' overrides.
    #[tokio::test]
    async fn read_all_skips_a_malformed_override() {
        let tmp = TempDir::new().unwrap();
        let dir = workspace_dir(&tmp);
        let ov = ServerOverride {
            start_command: Some("serve".to_string()),
            ..Default::default()
        };
        apply(&dir, "dev", &ov, false).await.unwrap();
        apply(&dir, "api", &ov, false).await.unwrap();
        fs::write(
            tmp.path().join(CONTROL_SUBDIR).join("api/override.json"),
            "{ not json",
        )
        .await
        .unwrap();

        let all = read_all(&dir).await;
        assert_eq!(all.keys().collect::<Vec<_>>(), vec!["dev"]);
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

        let meta = fs::metadata(tmp.path().join(CONTROL_SUBDIR).join("dev/env"))
            .await
            .unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
}
