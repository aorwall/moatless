//! Workspace setup: plugin fetches and setup commands.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use tokio::fs;
use tokio::process::Command;
use tracing::{info, warn};

use moatless_workspace_types::plugin::PluginDelivery;

use crate::backend_client::{BackendClient, PluginFetch};
use crate::config::Config;

/// Fetch each of the task owner's plugins from the backend into
/// [`Config::plugins_dir`] and remove the plugins and skills they did not pick.
///
/// Non-fatal per plugin: one the backend cannot serve keeps the restored copy,
/// pruned like a fresh one. Returns the plugins that are current.
pub async fn fetch_plugins(config: &Config, backend: &dyn BackendClient) -> Vec<String> {
    let base_dir = config.plugins_dir();
    for plugin in &config.plugins {
        restore_interrupted_install(&base_dir, &plugin.name);
    }
    remove_unpicked_plugins(&base_dir, &config.plugins).await;
    if config.plugins.is_empty() {
        return Vec::new();
    }

    if let Err(e) = fs::create_dir_all(&base_dir).await {
        warn!(error = %e, "Failed to create plugins directory");
        return Vec::new();
    }

    let mut fetched = Vec::new();
    for plugin in &config.plugins {
        let name = &plugin.name;
        let dest = base_dir.join(name);

        match fetch_plugin(config, backend, &base_dir, plugin).await {
            Ok(()) => fetched.push(dest.to_string_lossy().into_owned()),
            Err(e) if dest.exists() => warn!(
                name = %name,
                error = %e,
                "Failed to fetch plugin; keeping the copy restored from the snapshot"
            ),
            Err(e) => {
                warn!(name = %name, error = %e, "Failed to fetch plugin (non-fatal)");
                continue;
            }
        }

        let dropped = super::plugin_skills::keep_only(&dest, name, &plugin.skill_names);
        if dropped > 0 {
            info!(name = %name, dropped, "Removed skills this task's owner did not pick");
        }
    }

    fetched
}

/// What an installed plugin was fetched at, kept beside it as `.{name}.json`.
#[derive(Debug, Serialize, Deserialize)]
struct Installed {
    version: String,
    /// The picks it was pruned to; a skill outside them is no longer on disk.
    skills: Vec<String>,
}

fn installed_path(base_dir: &Path, name: &str) -> PathBuf {
    base_dir.join(format!(".{name}.json"))
}

/// The version to name as known: only one whose copy still holds every skill
/// now picked.
async fn known_version(base_dir: &Path, plugin: &PluginDelivery) -> Option<String> {
    if !base_dir.join(&plugin.name).is_dir() {
        return None;
    }
    let raw = fs::read(installed_path(base_dir, &plugin.name))
        .await
        .ok()?;
    let installed: Installed = serde_json::from_slice(&raw).ok()?;
    plugin
        .skill_names
        .iter()
        .all(|skill| installed.skills.contains(skill))
        .then_some(installed.version)
}

async fn fetch_plugin(
    config: &Config,
    backend: &dyn BackendClient,
    base_dir: &Path,
    plugin: &PluginDelivery,
) -> Result<(), String> {
    let name = &plugin.name;
    let known = known_version(base_dir, plugin).await;
    let fetched = backend
        .fetch_plugin(&config.task_id, name, known.as_deref())
        .await
        .map_err(|e| format!("{}: {}", e.code(), e.message()))?;
    let version = match fetched {
        PluginFetch::Unchanged => {
            info!(name = %name, version = ?known, "Plugin unchanged since it was fetched");
            known.unwrap_or_default()
        }
        PluginFetch::Archive { version, data } => {
            // Dropped first, so a failed install is never taken for `version`.
            let _ = fs::remove_file(installed_path(base_dir, name)).await;
            let (dir, owned) = (base_dir.to_path_buf(), name.clone());
            tokio::task::spawn_blocking(move || install_archive(&data, &dir, &owned))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| format!("Failed to unpack plugin archive: {e}"))?;
            info!(name = %name, version = %version, "Plugin fetched from the backend");
            version
        }
    };

    let installed = Installed {
        version,
        skills: plugin.skill_names.clone(),
    };
    let written = match serde_json::to_vec(&installed) {
        Ok(raw) => fs::write(installed_path(base_dir, name), raw)
            .await
            .map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    if let Err(e) = written {
        warn!(name = %name, error = %e, "Failed to record the plugin's version; the next start downloads it again");
    }
    Ok(())
}

/// Unpack a `FetchPlugin` archive to `base_dir/name`, replacing what is there
/// only once the archive has unpacked whole.
fn install_archive(data: &[u8], base_dir: &Path, name: &str) -> std::io::Result<()> {
    // Dot-prefixed so a leftover from a killed start is not loaded as a plugin;
    // `remove_unpicked_plugins` clears it on the next one.
    let staging = base_dir.join(format!(".{name}.partial"));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    let installed = unpack_into(
        data,
        &staging,
        &previous_path(base_dir, name),
        &base_dir.join(name),
    );
    if staging.exists() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    installed
}

/// Where [`unpack_into`] moves the installed copy while it promotes the new one.
fn previous_path(base_dir: &Path, name: &str) -> PathBuf {
    base_dir.join(format!(".{name}.previous"))
}

/// Put back the copy a start killed between [`unpack_into`]'s two renames left
/// aside, before `remove_unpicked_plugins` would delete it.
fn restore_interrupted_install(base_dir: &Path, name: &str) {
    let (previous, dest) = (previous_path(base_dir, name), base_dir.join(name));
    if dest.exists() || !previous.exists() {
        return;
    }
    match std::fs::rename(&previous, &dest) {
        Ok(()) => info!(name = %name, "Restored the plugin copy an interrupted install left aside"),
        Err(e) => {
            warn!(name = %name, error = %e, "Failed to restore the plugin copy an interrupted install left aside");
        }
    }
}

fn unpack_into(data: &[u8], staging: &Path, previous: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(staging)?;
    tar::Archive::new(flate2::read::GzDecoder::new(data)).unpack(staging)?;
    if !dest.exists() {
        return std::fs::rename(staging, dest);
    }
    // Moved aside rather than deleted, so a failed promotion can put it back.
    if previous.exists() {
        std::fs::remove_dir_all(previous)?;
    }
    std::fs::rename(dest, previous)?;
    if let Err(e) = std::fs::rename(staging, dest) {
        let _ = std::fs::rename(previous, dest);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(previous);
    Ok(())
}

/// Remove every child of `base_dir` that `picked` does not name.
///
/// The snapshot restores `base_dir` whole, and `agent::options::build_plugins`
/// loads every child of it, so a plugin dropped between starts would otherwise
/// come back with all its skills.
async fn remove_unpicked_plugins(base_dir: &Path, picked: &[PluginDelivery]) {
    let Ok(mut entries) = fs::read_dir(base_dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        if picked.iter().any(|plugin| {
            name == plugin.name.as_str()
                || installed_path(base_dir, &plugin.name).file_name() == Some(name.as_os_str())
        }) {
            continue;
        }
        let path = entry.path();
        let removed = if entry.file_type().await.is_ok_and(|ft| ft.is_dir()) {
            fs::remove_dir_all(&path).await
        } else {
            fs::remove_file(&path).await
        };
        match removed {
            Ok(()) => {
                info!(path = %path.display(), "Removed plugin the task's owner no longer picks");
            }
            Err(e) => warn!(path = %path.display(), error = %e, "Failed to remove unpicked plugin"),
        }
    }
}

/// Run a single setup command in the repository directory.
pub async fn run_setup_command(config: &Config, cmd: &str) -> Result<(), String> {
    if cmd.is_empty() {
        return Ok(());
    }

    info!(command = %cmd, "Running setup command");
    // bash, not `sh`: dash never reads the image's `BASH_ENV`, which exports
    // the `CodeArtifact` token a setup `bun install` needs.
    let output = Command::new("bash")
        .args(["-c", cmd])
        .current_dir(&config.repo_dir)
        .output()
        .await
        .map_err(|e| format!("Failed to execute: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Exit code {}: {stderr}", output.status));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    /// A gzip tar laid out as `FetchPlugin` serves one: rooted at the
    /// repository root.
    fn plugin_archive(files: &[(&str, &str)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (path, contents) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            builder
                .append_data(&mut header, path, contents.as_bytes())
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    /// Runs under bash so the image's `BASH_ENV` script reaches the command;
    /// `sh` is dash, which never reads it. `[[` fails under dash.
    #[tokio::test]
    async fn a_setup_command_runs_under_bash() {
        let root = tempdir().unwrap();
        let mut config = crate::config::test_support::minimal_config();
        config.repo_dir = root.path().to_string_lossy().into_owned();
        assert!(
            run_setup_command(&config, "[[ -n ${BASH_VERSION:-} ]]")
                .await
                .is_ok()
        );
    }

    #[test]
    fn an_archive_replaces_the_previous_copy_whole() {
        let root = tempdir().unwrap();
        let dest = root.path().join("agentops");
        std::fs::create_dir_all(dest.join("skills/stale")).unwrap();
        let data = plugin_archive(&[("skills/review/SKILL.md", "v2\n")]);

        install_archive(&data, root.path(), "agentops").unwrap();

        assert!(dest.join("skills/review/SKILL.md").is_file());
        assert!(!dest.join("skills/stale").exists());
        assert!(!root.path().join(".agentops.partial").exists());
    }

    #[test]
    fn a_corrupt_archive_leaves_the_previous_copy_in_place() {
        let root = tempdir().unwrap();
        let dest = root.path().join("agentops");
        std::fs::create_dir_all(dest.join("skills/review")).unwrap();

        assert!(install_archive(b"not a tarball", root.path(), "agentops").is_err());

        assert!(dest.join("skills/review").is_dir());
        assert!(!root.path().join(".agentops.partial").exists());
    }

    /// The snapshot restores every plugin a previous start fetched, including
    /// ones the owner has since dropped; those must not survive to be loaded,
    /// and a kept one the backend cannot serve is still pruned.
    #[tokio::test]
    async fn a_restored_plugin_the_owner_dropped_is_removed() {
        let root = tempdir().unwrap();
        let mut config = crate::config::test_support::minimal_config();
        config.workspace_dir = root.path().join("ws").to_string_lossy().into_owned();
        let plugins_dir = config.plugins_dir();
        std::fs::create_dir_all(plugins_dir.join("dropped/skills/x")).unwrap();
        std::fs::create_dir_all(plugins_dir.join("kept/skills/review")).unwrap();
        std::fs::create_dir_all(plugins_dir.join("kept/skills/lint")).unwrap();
        config.plugins = vec![PluginDelivery {
            name: "kept".to_string(),
            git_url: "https://github.com/acme/plugins".to_string(),
            skill_names: vec!["review".to_string()],
            frontload_skill_names: Vec::new(),
        }];
        let backend = crate::state::test_support::NoopBackend::default();

        assert!(fetch_plugins(&config, &backend).await.is_empty());
        assert!(plugins_dir.join("kept/skills/review").is_dir());
        assert!(!plugins_dir.join("kept/skills/lint").exists());
        assert!(!plugins_dir.join("dropped").exists());

        config.plugins = Vec::new();
        fetch_plugins(&config, &backend).await;
        assert!(!plugins_dir.join("kept").exists());
    }

    #[tokio::test]
    async fn a_copy_left_aside_by_a_killed_install_is_restored() {
        let root = tempdir().unwrap();
        let config = config_picking(root.path(), &["review"]);
        let plugins_dir = config.plugins_dir();
        std::fs::create_dir_all(plugins_dir.join(".agentops.previous/skills/review")).unwrap();
        let backend = crate::state::test_support::NoopBackend::default();

        assert!(fetch_plugins(&config, &backend).await.is_empty());

        assert!(plugins_dir.join("agentops/skills/review").is_dir());
        assert!(!plugins_dir.join(".agentops.previous").exists());
    }

    /// Serves `version` of a one-skill plugin, answering `Unchanged` to a
    /// caller that names it, and records what each call named.
    struct PluginBackend {
        version: &'static str,
        known: std::sync::Mutex<Vec<Option<String>>>,
    }

    #[async_trait::async_trait]
    impl BackendClient for PluginBackend {
        async fn fetch_plugin(
            &self,
            _task_id: &str,
            _plugin_name: &str,
            known_version: Option<&str>,
        ) -> Result<PluginFetch, tonic::Status> {
            self.known
                .lock()
                .unwrap()
                .push(known_version.map(str::to_string));
            if known_version == Some(self.version) {
                return Ok(PluginFetch::Unchanged);
            }
            Ok(PluginFetch::Archive {
                version: self.version.to_string(),
                data: plugin_archive(&[
                    ("skills/review/SKILL.md", self.version),
                    ("skills/lint/SKILL.md", self.version),
                ]),
            })
        }

        async fn publish_event(
            &self,
            _req: sandbox_proto::PublishEventRequest,
        ) -> Result<(), tonic::Status> {
            unimplemented!()
        }
        async fn get_pending_messages(
            &self,
            _task_id: &str,
        ) -> Result<Vec<sandbox_proto::PendingAgentDelivery>, tonic::Status> {
            unimplemented!()
        }
        async fn acknowledge_messages(
            &self,
            _task_id: &str,
            _message_uuids: Vec<String>,
            _session_id: &str,
            _run_id: &str,
            _answered_prompt: Option<&str>,
        ) -> Result<(), tonic::Status> {
            unimplemented!()
        }
        async fn get_credentials(
            &self,
            _task_id: &str,
        ) -> Result<sandbox_proto::GetCredentialsResponse, tonic::Status> {
            unimplemented!()
        }
        async fn update_credential(
            &self,
            _task_id: &str,
            _key: &str,
            _value: &str,
        ) -> Result<(), tonic::Status> {
            unimplemented!()
        }
        async fn get_system_prompt(&self, _task_id: &str) -> Result<String, tonic::Status> {
            unimplemented!()
        }
        async fn download_upload(
            &self,
            _task_id: &str,
            _file_id: &str,
        ) -> Result<crate::backend_client::DownloadedUpload, tonic::Status> {
            unimplemented!()
        }
        async fn recover_interrupted_run(
            &self,
            _task_id: &str,
        ) -> Result<Option<sandbox_proto::InterruptedTurn>, tonic::Status> {
            unimplemented!()
        }
        async fn upload_transcript(
            &self,
            _task_id: &str,
            _tar_data: Vec<u8>,
        ) -> Result<(), tonic::Status> {
            unimplemented!()
        }
        async fn download_transcript(&self, _task_id: &str) -> Result<Vec<u8>, tonic::Status> {
            unimplemented!()
        }
    }

    fn config_picking(root: &Path, skills: &[&str]) -> Config {
        let mut config = crate::config::test_support::minimal_config();
        config.workspace_dir = root.join("ws").to_string_lossy().into_owned();
        config.plugins = vec![PluginDelivery {
            name: "agentops".to_string(),
            git_url: "https://github.com/acme/plugins".to_string(),
            skill_names: skills.iter().map(ToString::to_string).collect(),
            frontload_skill_names: Vec::new(),
        }];
        config
    }

    #[tokio::test]
    async fn a_plugin_still_at_its_version_is_not_downloaded_again() {
        let root = tempdir().unwrap();
        let config = config_picking(root.path(), &["review"]);
        let backend = PluginBackend {
            version: "sha-1",
            known: Default::default(),
        };

        fetch_plugins(&config, &backend).await;
        let fetched = fetch_plugins(&config, &backend).await;

        assert_eq!(fetched.len(), 1);
        assert_eq!(
            *backend.known.lock().unwrap(),
            [None, Some("sha-1".to_string())]
        );
        let skills = config.plugins_dir().join("agentops/skills");
        assert!(skills.join("review/SKILL.md").is_file());
        assert!(!skills.join("lint").exists());
    }

    #[tokio::test]
    async fn a_newly_picked_skill_downloads_the_plugin_again() {
        let root = tempdir().unwrap();
        let backend = PluginBackend {
            version: "sha-1",
            known: Default::default(),
        };
        fetch_plugins(&config_picking(root.path(), &["review"]), &backend).await;

        let config = config_picking(root.path(), &["review", "lint"]);
        fetch_plugins(&config, &backend).await;

        assert_eq!(*backend.known.lock().unwrap(), [None, None]);
        assert!(config.plugins_dir().join("agentops/skills/lint").is_dir());
    }
}
