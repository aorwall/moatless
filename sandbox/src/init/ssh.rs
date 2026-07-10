//! SSH private key materialization.
//!
//! SSH keys arrive from the backend on a dedicated channel (separate from env
//! credentials) so they are never exposed as environment variables to the
//! agent. They are written into `~/.ssh` (mode 0600) and wired via an
//! `~/.ssh/config` entry so `git` and `ssh` use them transparently for SSH
//! remotes — the agent uses the key for cloning/pushing/ssh without ever
//! reading its value.

use std::collections::HashMap;

use tokio::fs;
use tracing::{info, warn};

use crate::config::Config;

/// `~/.ssh` must be 0700 and its contents 0600, or ssh refuses to use the keys.
/// The sandbox home is ephemeral, so we own the whole `~/.ssh/config` file (no
/// user-authored ssh config to preserve).
const SSH_DIR_MODE: u32 = 0o700;
const KEY_FILE_MODE: u32 = 0o600;

/// Write each SSH private key to `~/.ssh` and generate an `~/.ssh/config` that
/// makes them the identities for all hosts (trust-on-first-use host checking,
/// since SSH remotes can be arbitrary).
///
/// No-op when `ssh_keys` is empty. Idempotent: safe to call again on the
/// periodic credential refresh.
pub async fn materialize_ssh_keys(config: &Config, ssh_keys: &HashMap<String, String>) {
    if ssh_keys.is_empty() {
        return;
    }

    let ssh_dir = format!("{}/.ssh", config.home_dir);
    if let Err(e) = fs::create_dir_all(&ssh_dir).await {
        warn!(error = %e, dir = %ssh_dir, "Failed to create ~/.ssh directory");
        return;
    }
    set_mode(&ssh_dir, SSH_DIR_MODE).await;

    // Deterministic ordering so the config file (and any diff/logging) is stable.
    let mut names: Vec<&String> = ssh_keys.keys().collect();
    names.sort();

    let mut identity_files: Vec<String> = Vec::with_capacity(names.len());
    for name in names {
        let Some(value) = ssh_keys.get(name) else {
            continue;
        };
        let file_name = key_file_name(name);
        let key_path = format!("{ssh_dir}/{file_name}");
        // SSH refuses keys without a trailing newline.
        let contents = if value.ends_with('\n') {
            value.clone()
        } else {
            format!("{value}\n")
        };
        match fs::write(&key_path, contents).await {
            Ok(()) => {
                set_mode(&key_path, KEY_FILE_MODE).await;
                info!(secret = %name, path = %key_path, "Wrote SSH private key");
                identity_files.push(key_path);
            }
            Err(e) => {
                warn!(error = %e, secret = %name, "Failed to write SSH private key");
            }
        }
    }

    if identity_files.is_empty() {
        return;
    }

    // known_hosts must exist for accept-new to append to it.
    let known_hosts = format!("{ssh_dir}/known_hosts");
    if fs::metadata(&known_hosts).await.is_err() {
        if let Err(e) = fs::write(&known_hosts, "").await {
            warn!(error = %e, "Failed to create ~/.ssh/known_hosts");
        } else {
            set_mode(&known_hosts, KEY_FILE_MODE).await;
        }
    }

    let config_path = format!("{ssh_dir}/config");
    let config_body = render_ssh_config(&identity_files, &known_hosts);
    match fs::write(&config_path, config_body).await {
        Ok(()) => {
            set_mode(&config_path, KEY_FILE_MODE).await;
            info!(keys = identity_files.len(), "Configured SSH identities for git/ssh");
        }
        Err(e) => {
            warn!(error = %e, "Failed to write ~/.ssh/config");
        }
    }
}

/// Filename under `~/.ssh` for a given secret key name, e.g.
/// `GIT_SSH_PRIVATE_KEY` → `moatless_git_ssh_private_key`.
fn key_file_name(secret_name: &str) -> String {
    let sanitized: String = secret_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect();
    format!("moatless_{sanitized}")
}

/// Render an `~/.ssh/config` that applies the given identities to all hosts.
///
/// `IdentitiesOnly yes` prevents ssh from offering unrelated keys (avoids
/// "Too many authentication failures"). `StrictHostKeyChecking accept-new`
/// trusts a host key on first use — appropriate for the ephemeral sandbox
/// talking to arbitrary SSH remotes.
fn render_ssh_config(identity_files: &[String], known_hosts: &str) -> String {
    let mut out = String::from("Host *\n");
    out.push_str("    StrictHostKeyChecking accept-new\n");
    out.push_str(&format!("    UserKnownHostsFile {known_hosts}\n"));
    out.push_str("    IdentitiesOnly yes\n");
    for path in identity_files {
        out.push_str(&format!("    IdentityFile {path}\n"));
    }
    out
}

#[cfg(unix)]
async fn set_mode(path: &str, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).await {
        warn!(error = %e, path = %path, "Failed to set permissions");
    }
}

#[cfg(not(unix))]
async fn set_mode(_path: &str, _mode: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_file_name_sanitizes() {
        assert_eq!(key_file_name("GIT_SSH_PRIVATE_KEY"), "moatless_git_ssh_private_key");
        assert_eq!(key_file_name("DEPLOY-KEY.1"), "moatless_deploy_key_1");
    }

    #[test]
    fn render_config_lists_all_identities() {
        let cfg = render_ssh_config(
            &["/home/m/.ssh/moatless_a".to_string(), "/home/m/.ssh/moatless_b".to_string()],
            "/home/m/.ssh/known_hosts",
        );
        assert!(cfg.contains("Host *"));
        assert!(cfg.contains("StrictHostKeyChecking accept-new"));
        assert!(cfg.contains("IdentitiesOnly yes"));
        assert!(cfg.contains("IdentityFile /home/m/.ssh/moatless_a"));
        assert!(cfg.contains("IdentityFile /home/m/.ssh/moatless_b"));
        assert!(cfg.contains("UserKnownHostsFile /home/m/.ssh/known_hosts"));
    }

    #[tokio::test]
    async fn materialize_writes_keys_and_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = crate::config::test_support::minimal_config();
        config.home_dir = tmp.path().display().to_string();

        let mut keys = HashMap::new();
        keys.insert("GIT_SSH_PRIVATE_KEY".to_string(), "PRIVATE-KEY-BODY".to_string());
        materialize_ssh_keys(&config, &keys).await;

        let key_path = tmp.path().join(".ssh/moatless_git_ssh_private_key");
        let body = tokio::fs::read_to_string(&key_path).await.unwrap();
        assert_eq!(body, "PRIVATE-KEY-BODY\n", "key must end with a newline");

        let cfg = tokio::fs::read_to_string(tmp.path().join(".ssh/config"))
            .await
            .unwrap();
        assert!(cfg.contains("IdentityFile"));
        assert!(cfg.contains("moatless_git_ssh_private_key"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = tokio::fs::metadata(&key_path).await.unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
    }

    #[tokio::test]
    async fn materialize_is_noop_when_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = crate::config::test_support::minimal_config();
        config.home_dir = tmp.path().display().to_string();
        materialize_ssh_keys(&config, &HashMap::new()).await;
        assert!(!tmp.path().join(".ssh").exists());
    }
}
