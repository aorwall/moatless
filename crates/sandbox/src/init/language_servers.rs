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

/// Bridges Claude's stdio to the sandbox's shared server; see `code_intel`.
const PROXY_COMMAND: &str = "moat";

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
    /// `initializationOptions` for a server rooted at the given directory.
    pub(crate) initialization_options: fn(&Path) -> Value,
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
            initialization_options: |_| Value::Null,
        },
        LanguageServer::Rust => ServerSpec {
            command: "rust-analyzer",
            args: &[],
            extensions: &[(".rs", "rust")],
            initialization_options: |_| rust_analyzer_options(),
        },
        // `docker/features/java/moatless-jdtls` adds the CodeArtifact token,
        // Lombok and a data directory to JDT.LS.
        LanguageServer::Java => ServerSpec {
            command: "moatless-jdtls",
            args: &[],
            extensions: &[(".java", "java")],
            initialization_options: jdtls_options,
        },
    }
}

/// rust-analyzer settings that keep it inside a sandbox's memory. On this repo
/// its defaults hold 4.8 GiB, and 7.4 GiB with the `cargo check` that
/// `checkOnSave` runs beside the agent's own build; these hold 2.7 GiB. Lookups
/// load crates as they reach them rather than priming all at once.
fn rust_analyzer_options() -> Value {
    json!({
        "cachePriming": { "enable": false },
        "checkOnSave": false,
    })
}

/// JDT.LS settings for navigation. Without `autobuild` off it compiles into
/// each module's `target/classes`, the directory the agent's `mvn` builds
/// into; `moatless-jdtls` keeps its Eclipse metadata out of the checkout.
fn jdtls_options(root: &Path) -> Value {
    let mut java = json!({
        "autobuild": { "enabled": false },
        "maven": { "downloadSources": false },
        "eclipse": { "downloadSources": false },
    });
    if let Some(settings) = maven_user_settings(root) {
        java["configuration"] = json!({ "maven": { "userSettings": settings } });
    }
    json!({
        "settings": { "java": java },
        // Without it, a definition inside a jar or the JDK comes back empty.
        "extendedClientCapabilities": {
            "classFileContentsSupport": true,
            "progressReportProvider": false,
        },
    })
}

/// The settings file a repository's `.mvn/maven.config` passes with `-s`, which
/// `mvn` applies and JDT.LS's embedded Maven does not. Looks at `root`, then
/// each directory directly under it in name order; the first file found applies
/// to every checkout, since JDT.LS takes one settings file per server.
fn maven_user_settings(root: &Path) -> Option<PathBuf> {
    let children = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path());
    let mut projects: Vec<PathBuf> = std::iter::once(root.to_path_buf())
        .chain(children)
        .collect();
    projects[1..].sort();
    projects.into_iter().find_map(|project| {
        let config = std::fs::read_to_string(project.join(".mvn/maven.config")).ok()?;
        let mut args = config.split_whitespace();
        while let Some(arg) = args.next() {
            let settings = match arg {
                "-s" | "--settings" => args.next(),
                _ => arg
                    .strip_prefix("--settings=")
                    .or_else(|| arg.strip_prefix("-s").filter(|rest| !rest.is_empty())),
            };
            if let Some(settings) = settings {
                return Some(project.join(settings));
            }
        }
        None
    })
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
/// A server whose command is not on `search_path` is left out with a warning,
/// or every `LSP` call on its files would fail. Each is reached through
/// [`PROXY_COMMAND`] and its socket under `runtime_dir`, or started directly
/// without `moat`. Returns the servers written; none written leaves no plugin.
pub(super) fn stage_plugin(
    claude_config_dir: &Path,
    runtime_dir: &Path,
    servers: &[LanguageServer],
    search_path: Option<&OsStr>,
) -> io::Result<Vec<LanguageServer>> {
    let dir = plugin_dir(claude_config_dir);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }

    let proxied = on_path(PROXY_COMMAND, search_path);
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
        let (command, args) = if proxied {
            let socket = crate::code_intel::socket_path(runtime_dir, server);
            (PROXY_COMMAND, json!(["lsp-proxy", socket]))
        } else {
            (spec.command, json!(spec.args))
        };
        lsp.insert(
            server.as_str().to_string(),
            json!({
                "command": command,
                "args": args,
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
            Path::new("/run/moatless"),
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
            Path::new("/run/moatless"),
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

        let staged = stage_plugin(
            config.path(),
            Path::new("/run/moatless"),
            &[LanguageServer::Rust],
            Some(&search_path),
        )
        .unwrap();

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
            Path::new("/run/moatless"),
            &[LanguageServer::Rust],
            Some(bin.path().as_os_str()),
        )
        .unwrap();

        let staged = stage_plugin(
            config.path(),
            Path::new("/run/moatless"),
            &[],
            Some(bin.path().as_os_str()),
        )
        .unwrap();

        assert!(staged.is_empty());
        assert!(plugin_dir_if_staged(config.path()).is_none());
        assert!(!plugin_dir(config.path()).exists());
    }

    #[test]
    fn with_moat_on_path_each_server_is_reached_through_its_socket() {
        let config = TempDir::new().unwrap();
        let bin = TempDir::new().unwrap();
        fake_bin(bin.path(), "moat");
        fake_bin(bin.path(), "rust-analyzer");

        stage_plugin(
            config.path(),
            Path::new("/run/moatless"),
            &[LanguageServer::Rust],
            Some(bin.path().as_os_str()),
        )
        .unwrap();

        let lsp: Value = serde_json::from_slice(
            &std::fs::read(plugin_dir(config.path()).join(".lsp.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(lsp["rust"]["command"], "moat");
        assert_eq!(
            lsp["rust"]["args"],
            json!(["lsp-proxy", "/run/moatless/lsp/rust.sock"])
        );
    }

    #[test]
    fn rust_analyzer_neither_primes_every_crate_nor_checks_on_save() {
        let options = (server_spec(LanguageServer::Rust).initialization_options)(Path::new("/"));
        assert_eq!(options["cachePriming"]["enable"], false);
        assert_eq!(options["checkOnSave"], false);
    }

    #[test]
    fn jdtls_gets_the_settings_file_a_checkout_passes_to_maven() {
        let workspace = TempDir::new().unwrap();
        let repo = workspace.path().join("soap-backend");
        std::fs::create_dir_all(repo.join(".mvn")).unwrap();
        std::fs::write(
            repo.join(".mvn/maven.config"),
            "-T 1C\n--settings .github/ci-settings.xml\n",
        )
        .unwrap();

        let options = jdtls_options(workspace.path());

        assert_eq!(
            options["settings"]["java"]["configuration"]["maven"]["userSettings"],
            json!(repo.join(".github/ci-settings.xml"))
        );
        assert_eq!(options["settings"]["java"]["autobuild"]["enabled"], false);
        assert_eq!(
            options["extendedClientCapabilities"]["classFileContentsSupport"],
            true
        );
    }

    #[test]
    fn the_short_settings_flag_is_read_in_both_forms_and_absent_is_none() {
        let dir = TempDir::new().unwrap();
        assert_eq!(maven_user_settings(dir.path()), None);
        std::fs::create_dir(dir.path().join(".mvn")).unwrap();
        std::fs::write(dir.path().join(".mvn/maven.config"), "-sci.xml").unwrap();
        assert_eq!(
            maven_user_settings(dir.path()),
            Some(dir.path().join("ci.xml"))
        );
        std::fs::write(dir.path().join(".mvn/maven.config"), "-s ci.xml").unwrap();
        assert_eq!(
            maven_user_settings(dir.path()),
            Some(dir.path().join("ci.xml"))
        );
    }
}
