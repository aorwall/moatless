//! The per-checkout language servers behind [`CodeIntel`], and how their
//! answers become [`Location`]s.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use moatless_workspace_types::workspace::LanguageServer;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::info;

use super::lsp_client::{LspClient, LspError, path_to_uri, uri_to_path};
use crate::files::{FileError, validate_path};
use crate::init::language_servers::server_spec;

/// Servers the UI may start. `rust` is left out deliberately: Claude's plugin
/// already runs one rust-analyzer per checkout, and a second copy (about 5.7 GB
/// on the moatless repo) does not fit beside it in a 12 GiB sandbox.
const UI_SERVERS: &[LanguageServer] = &[LanguageServer::Typescript];

const IDLE_TIMEOUT: Duration = Duration::from_mins(15);
const START_TIMEOUT: Duration = Duration::from_secs(30);
/// A first lookup loads the file's whole project, which takes seconds on a large
/// monorepo; later ones answer in milliseconds.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// A definition the UI can open: a path relative to the workspace directory,
/// with 1-based lines and 1-based UTF-16 columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum CodeIntelError {
    #[error(transparent)]
    File(#[from] FileError),
    /// No configured server handles this file's extension.
    #[error("no language server is configured for {0}")]
    Unsupported(String),
    #[error(transparent)]
    Server(#[from] LspError),
}

struct Server {
    client: LspClient,
    /// Text last sent per open document, so an unchanged file is not resent.
    documents: Mutex<HashMap<PathBuf, (i32, String)>>,
    last_used: std::sync::Mutex<Instant>,
}

type ServerKey = (PathBuf, LanguageServer);

/// The servers this pod runs for the UI, keyed by workspace directory and
/// language.
#[derive(Clone)]
pub struct CodeIntel {
    enabled: Vec<LanguageServer>,
    servers: Arc<Mutex<HashMap<ServerKey, Arc<Server>>>>,
    cancel: CancellationToken,
}

impl CodeIntel {
    /// `configured` is the Workspace's `languageServers`; only those in
    /// [`UI_SERVERS`] are used.
    pub fn new(configured: &[LanguageServer], cancel: CancellationToken) -> Self {
        Self {
            enabled: configured
                .iter()
                .copied()
                .filter(|server| UI_SERVERS.contains(server))
                .collect(),
            servers: Arc::default(),
            cancel,
        }
    }

    /// Where the symbol at `line`/`column` (1-based, UTF-16) of `path` is
    /// defined. Definitions outside `workspace_dir`, such as the standard
    /// library's declarations, are left out: the UI cannot open them.
    pub async fn definition(
        &self,
        workspace_dir: &str,
        path: &str,
        line: u32,
        column: u32,
    ) -> Result<Vec<Location>, CodeIntelError> {
        let full_path = validate_path(workspace_dir, path)?;
        let (language, language_id) = self
            .server_for(&full_path)
            .ok_or_else(|| CodeIntelError::Unsupported(path.to_string()))?;
        let text = tokio::fs::read_to_string(&full_path)
            .await
            .map_err(FileError::Io)?;
        let root = PathBuf::from(workspace_dir);
        let server = self.server(&root, language).await?;
        let uri = server.sync(&full_path, language_id, text).await?;

        let result = server
            .client
            .request(
                "textDocument/definition",
                json!({
                    "textDocument": { "uri": uri },
                    "position": {
                        "line": line.saturating_sub(1),
                        "character": column.saturating_sub(1),
                    },
                }),
                REQUEST_TIMEOUT,
            )
            .await;
        let result = match result {
            Err(LspError::Exited) => {
                self.servers.lock().await.remove(&(root.clone(), language));
                return Err(LspError::Exited.into());
            }
            other => other?,
        };
        Ok(locations(&result, &root))
    }

    fn server_for(&self, path: &Path) -> Option<(LanguageServer, &'static str)> {
        let extension = format!(".{}", path.extension()?.to_str()?);
        self.enabled.iter().find_map(|&server| {
            server_spec(server)
                .extensions
                .iter()
                .find(|(ext, _)| *ext == extension)
                .map(|(_, language_id)| (server, *language_id))
        })
    }

    async fn server(&self, root: &Path, language: LanguageServer) -> Result<Arc<Server>, LspError> {
        let key = (root.to_path_buf(), language);
        // Held across the start so two first lookups do not start two servers.
        let mut servers = self.servers.lock().await;
        if let Some(server) = servers.get(&key) {
            server.touch();
            return Ok(Arc::clone(server));
        }
        let spec = server_spec(language);
        let started = Instant::now();
        let client = LspClient::start(spec.command, spec.args, root, START_TIMEOUT).await?;
        info!(server = %language, root = %root.display(), elapsed_ms = started.elapsed().as_millis() as u64, "Started language server for the UI");
        let server = Arc::new(Server {
            client,
            documents: Mutex::default(),
            last_used: std::sync::Mutex::new(Instant::now()),
        });
        servers.insert(key.clone(), Arc::clone(&server));
        tokio::spawn(reap_when_idle(
            Arc::clone(&self.servers),
            key,
            Arc::downgrade(&server),
            self.cancel.clone(),
        ));
        Ok(server)
    }
}

impl Server {
    fn touch(&self) {
        if let Ok(mut last_used) = self.last_used.lock() {
            *last_used = Instant::now();
        }
    }

    fn idle_for(&self) -> Duration {
        self.last_used
            .lock()
            .map_or(Duration::ZERO, |last_used| last_used.elapsed())
    }

    /// Open `path` with `text`, or send the new text when it changed since the
    /// last lookup. Returns the document's URI.
    async fn sync(&self, path: &Path, language_id: &str, text: String) -> Result<String, LspError> {
        let uri = path_to_uri(path);
        let mut documents = self.documents.lock().await;
        match documents.get_mut(path) {
            Some((_, sent)) if *sent == text => {}
            Some((version, sent)) => {
                *version += 1;
                self.client
                    .notify(
                        "textDocument/didChange",
                        json!({
                            "textDocument": { "uri": uri, "version": *version },
                            "contentChanges": [{ "text": text }],
                        }),
                    )
                    .await?;
                *sent = text;
            }
            None => {
                self.client
                    .notify(
                        "textDocument/didOpen",
                        json!({
                            "textDocument": {
                                "uri": uri,
                                "languageId": language_id,
                                "version": 1,
                                "text": text,
                            },
                        }),
                    )
                    .await?;
                documents.insert(path.to_path_buf(), (1, text));
            }
        }
        Ok(uri)
    }
}

/// Drop the server once it has gone [`IDLE_TIMEOUT`] without a lookup, or when
/// the pod shuts down; dropping it kills the child.
async fn reap_when_idle(
    servers: Arc<Mutex<HashMap<ServerKey, Arc<Server>>>>,
    key: ServerKey,
    server: std::sync::Weak<Server>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            () = tokio::time::sleep(Duration::from_mins(1)) => {}
        }
        let Some(live) = server.upgrade() else { return };
        if live.idle_for() >= IDLE_TIMEOUT {
            break;
        }
    }
    let mut servers = servers.lock().await;
    // A server that exited was already replaced under the same key.
    if servers
        .get(&key)
        .is_some_and(|current| std::ptr::eq(Arc::as_ptr(current), server.as_ptr()))
    {
        servers.remove(&key);
        info!(server = %key.1, root = %key.0.display(), "Stopped idle language server");
    }
}

/// Normalize a `textDocument/definition` result (`Location`, `Location[]` or
/// `LocationLink[]`) to locations inside `root`.
fn locations(result: &Value, root: &Path) -> Vec<Location> {
    let items = match result {
        Value::Array(items) => items.clone(),
        Value::Null => Vec::new(),
        single => vec![single.clone()],
    };
    items
        .iter()
        .filter_map(|item| {
            // A `LocationLink` names the whole declaration in `targetRange` and
            // the name itself in `targetSelectionRange`, which is what to reveal.
            let (uri, range) = match item.get("targetUri") {
                Some(uri) => (uri, item.get("targetSelectionRange")?),
                None => (item.get("uri")?, item.get("range")?),
            };
            let path = uri_to_path(uri.as_str()?)?;
            let relative = path.strip_prefix(root).ok()?.to_str()?.to_string();
            let position = |key: &str, field: &str| {
                range
                    .pointer(&format!("/{key}/{field}"))
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
                    .map(|n| n + 1)
            };
            Some(Location {
                path: relative,
                line: position("start", "line")?,
                column: position("start", "character")?,
                end_line: position("end", "line")?,
                end_column: position("end", "character")?,
            })
        })
        .collect()
}

impl std::fmt::Debug for CodeIntel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeIntel")
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;
    use tracing::warn;

    #[test]
    fn location_links_reveal_the_name_and_outside_paths_are_dropped() {
        let result = json!([
            {
                "targetUri": "file:///workspace/repo/src/a.ts",
                "targetRange": { "start": { "line": 9, "character": 0 }, "end": { "line": 20, "character": 1 } },
                "targetSelectionRange": { "start": { "line": 9, "character": 13 }, "end": { "line": 9, "character": 20 } },
            },
            {
                "uri": "file:///usr/lib/node_modules/typescript/lib/lib.es5.d.ts",
                "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 5 } },
            },
        ]);

        assert_eq!(
            locations(&result, Path::new("/workspace")),
            [Location {
                path: "repo/src/a.ts".to_string(),
                line: 10,
                column: 14,
                end_line: 10,
                end_column: 21,
            }]
        );
    }

    #[test]
    fn a_single_location_and_null_are_accepted() {
        let single = json!({
            "uri": "file:///workspace/b.ts",
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } },
        });
        assert_eq!(locations(&single, Path::new("/workspace")).len(), 1);
        assert!(locations(&Value::Null, Path::new("/workspace")).is_empty());
    }

    #[test]
    fn only_ui_servers_are_enabled_and_matched_by_extension() {
        let intel = CodeIntel::new(
            &[LanguageServer::Rust, LanguageServer::Typescript],
            CancellationToken::new(),
        );
        assert_eq!(intel.enabled, [LanguageServer::Typescript]);
        assert_eq!(
            intel.server_for(Path::new("/w/a.tsx")),
            Some((LanguageServer::Typescript, "typescriptreact"))
        );
        assert!(intel.server_for(Path::new("/w/lib.rs")).is_none());
        assert!(intel.server_for(Path::new("/w/Makefile")).is_none());
    }

    /// Runs the real `tsc --lsp` when it is installed; the sandbox image has it.
    #[tokio::test]
    async fn typescript_definition_resolves_across_files() {
        let Some(path) = std::env::var_os("PATH") else {
            return;
        };
        if !std::env::split_paths(&path).any(|dir| dir.join("tsc").is_file()) {
            warn!("tsc not on PATH; skipping");
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("tsconfig.json"),
            r#"{"compilerOptions":{"strict":true}}"#,
        )
        .unwrap();
        std::fs::write(root.join("a.ts"), "export class Greeter {\n  hi() {}\n}\n").unwrap();
        std::fs::write(
            root.join("b.ts"),
            "import { Greeter } from \"./a\";\nnew Greeter().hi();\n",
        )
        .unwrap();

        let intel = CodeIntel::new(&[LanguageServer::Typescript], CancellationToken::new());
        let found = intel
            .definition(root.to_str().unwrap(), "b.ts", 2, 6)
            .await
            .unwrap();

        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].path, "a.ts");
        assert_eq!((found[0].line, found[0].column), (1, 14));
    }
}
