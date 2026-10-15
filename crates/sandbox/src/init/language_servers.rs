//! The Claude Code plugin that gives the agent its `LSP` tool.
//!
//! Generated from the Workspace's `languageServers` at every init, under
//! `$CLAUDE_CONFIG_DIR` rather than the workspace, and passed to Claude as one
//! more `--plugin-dir` by [`plugin_dir_if_staged`]. Codex has no LSP client.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use moatless_workspace_types::workspace::LanguageServer;
use serde_json::{Map, Value, json};
use tracing::{info, warn};

/// Directory name of the generated plugin under `$CLAUDE_CONFIG_DIR`.
/// `agent::options` tests stage a plugin under this name by hand.
const PLUGIN_DIR_NAME: &str = "moatless-language-servers";

/// How one language server is started, by Claude Code and by `code_intel`.
pub(crate) struct ServerSpec {
    /// Looked up on `PATH`, so a custom image can supply its own build.
    pub(crate) command: &'static str,
    pub(crate) args: &'static [&'static str],
    /// File extension to LSP language id.
    pub(crate) extensions: &'static [(&'static str, &'static str)],
}

pub(crate) fn server_spec(server: LanguageServer) -> ServerSpec {
    match server {
        // TypeScript 7's native server. It answers diagnostics only when pulled,
        // which Claude Code never does, so it navigates but reports no errors.
        LanguageServer::Typescript => ServerSpec {
            command: "tsc",
            args: &["--lsp", "--stdio"],
            extensions: &[
                (".ts", "typescript"),
                (".mts", "typescript"),
                (".cts", "typescript"),
                (".tsx", "typescriptreact"),
                (".js", "javascript"),
                (".mjs", "javascript"),
                (".cjs", "javascript"),
                (".jsx", "javascriptreact"),
            ],
        },
        LanguageServer::Rust => ServerSpec {
            command: "rust-analyzer",
            args: &[],
            extensions: &[(".rs", "rust")],
        },
    }
}

fn plugin_dir(claude_config_dir: &Path) -> PathBuf {
    claude_config_dir.join(PLUGIN_DIR_NAME)
}

/// The plugin directory when init staged one, for Claude's `--plugin-dir`.
pub(crate) fn plugin_dir_if_staged(claude_config_dir: &Path) -> Option<PathBuf> {
    let dir = plugin_dir(claude_config_dir);
    dir.join(".lsp.json").is_file().then_some(dir)
}

/// Replace the generated plugin with one declaring `servers`.
///
/// Servers whose command is not on `search_path` are left out with a warning:
/// declared anyway, every `LSP` call on their files would fail. Returns the
/// servers written; none written leaves no plugin.
pub(super) fn stage_plugin(
    claude_config_dir: &Path,
    servers: &[LanguageServer],
    search_path: Option<&OsStr>,
) -> io::Result<Vec<LanguageServer>> {
    let dir = plugin_dir(claude_config_dir);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }

    let mut staged = Vec::new();
    let mut lsp = Map::new();
    for &server in servers {
        let spec = server_spec(server);
        if !on_path(spec.command, search_path) {
            warn!(server = %server, command = spec.command, "Language server not on PATH; leaving it out");
            continue;
        }
        let extensions: Map<String, Value> = spec
            .extensions
            .iter()
            .map(|(ext, lang)| ((*ext).to_string(), json!(lang)))
            .collect();
        lsp.insert(
            server.as_str().to_string(),
            json!({
                "command": spec.command,
                "args": spec.args,
                "extensionToLanguage": extensions,
            }),
        );
        staged.push(server);
    }
    if staged.is_empty() {
        return Ok(staged);
    }

    std::fs::create_dir_all(dir.join(".claude-plugin"))?;
    let manifest = json!({
        "name": PLUGIN_DIR_NAME,
        "description": "Language servers configured on this Workspace",
    });
    std::fs::write(
        dir.join(".claude-plugin/plugin.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    std::fs::write(
        dir.join(".lsp.json"),
        serde_json::to_vec_pretty(&Value::Object(lsp))?,
    )?;
    info!(servers = ?staged, dir = %dir.display(), "Staged language server plugin");
    Ok(staged)
}

/// Whether the first `command` on `search_path`, the one Claude will start, is
/// a real binary. A rustup proxy counts as missing: `rustup update` recreates
/// `/opt/cargo/bin/rust-analyzer`, which shadows the image's binary and exits
/// with "not installed for toolchain" unless the component is installed.
fn on_path(command: &str, search_path: Option<&OsStr>) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Some(paths) = search_path else {
        return false;
    };
    let first = std::env::split_paths(paths)
        .map(|dir| dir.join(command))
        .find(|path| {
            std::fs::metadata(path)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        });
    first.is_some_and(|path| {
        std::fs::canonicalize(&path)
            .is_ok_and(|real| real.file_name().is_none_or(|name| name != "rustup"))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test fixtures")]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;

    use super::*;

    fn fake_bin(dir: &Path, name: &str) {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn writes_a_plugin_declaring_each_installed_server() {
        let config = TempDir::new().unwrap();
        let bin = TempDir::new().unwrap();
        fake_bin(bin.path(), "tsc");
        fake_bin(bin.path(), "rust-analyzer");

        let staged = stage_plugin(
            config.path(),
            &[LanguageServer::Typescript, LanguageServer::Rust],
            Some(bin.path().as_os_str()),
        )
        .unwrap();

        assert_eq!(staged, [LanguageServer::Typescript, LanguageServer::Rust]);
        let dir = plugin_dir_if_staged(config.path()).unwrap();
        let lsp: Value =
            serde_json::from_slice(&std::fs::read(dir.join(".lsp.json")).unwrap()).unwrap();
        assert_eq!(lsp["typescript"]["command"], "tsc");
        assert_eq!(lsp["typescript"]["args"], json!(["--lsp", "--stdio"]));
        assert_eq!(
            lsp["typescript"]["extensionToLanguage"][".tsx"],
            "typescriptreact"
        );
        assert_eq!(lsp["rust"]["extensionToLanguage"], json!({".rs": "rust"}));
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(dir.join(".claude-plugin/plugin.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["name"], PLUGIN_DIR_NAME);
    }

    #[test]
    fn a_server_missing_from_path_is_left_out() {
        let config = TempDir::new().unwrap();
        let bin = TempDir::new().unwrap();
        fake_bin(bin.path(), "tsc");

        let staged = stage_plugin(
            config.path(),
            &[LanguageServer::Typescript, LanguageServer::Rust],
            Some(bin.path().as_os_str()),
        )
        .unwrap();

        assert_eq!(staged, [LanguageServer::Typescript]);
        let lsp: Value = serde_json::from_slice(
            &std::fs::read(plugin_dir(config.path()).join(".lsp.json")).unwrap(),
        )
        .unwrap();
        assert!(lsp.get("rust").is_none());
    }

    #[test]
    fn a_rustup_proxy_first_on_path_counts_as_missing() {
        let config = TempDir::new().unwrap();
        let cargo_bin = TempDir::new().unwrap();
        let local_bin = TempDir::new().unwrap();
        fake_bin(cargo_bin.path(), "rustup");
        std::os::unix::fs::symlink("rustup", cargo_bin.path().join("rust-analyzer")).unwrap();
        fake_bin(local_bin.path(), "rust-analyzer");
        let search_path = std::env::join_paths([cargo_bin.path(), local_bin.path()]).unwrap();

        let staged =
            stage_plugin(config.path(), &[LanguageServer::Rust], Some(&search_path)).unwrap();

        assert!(staged.is_empty());
    }

    /// A Workspace that drops its last server must not keep the plugin a
    /// previous init wrote, or the agent keeps starting it.
    #[test]
    fn staging_nothing_removes_an_earlier_plugin() {
        let config = TempDir::new().unwrap();
        let bin = TempDir::new().unwrap();
        fake_bin(bin.path(), "rust-analyzer");
        stage_plugin(
            config.path(),
            &[LanguageServer::Rust],
            Some(bin.path().as_os_str()),
        )
        .unwrap();

        let staged = stage_plugin(config.path(), &[], Some(bin.path().as_os_str())).unwrap();

        assert!(staged.is_empty());
        assert!(plugin_dir_if_staged(config.path()).is_none());
        assert!(!plugin_dir(config.path()).exists());
    }
}
