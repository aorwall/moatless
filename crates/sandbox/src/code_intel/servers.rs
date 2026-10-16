//! The per-checkout language servers behind [`CodeIntel`], and how their
//! answers become [`Location`]s.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use moatless_workspace_types::workspace::LanguageServer;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::lsp_client::{LspClient, LspError, path_to_uri, uri_to_path};
use crate::files::{FileError, validate_path};
use crate::init::language_servers::server_spec;

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
    /// The server answered nothing while reporting work in progress.
    #[error("{0} is still indexing the workspace; try again in a minute")]
    Indexing(LanguageServer),
    #[error(transparent)]
    Server(#[from] LspError),
}

pub(super) struct Server {
    pub(super) client: LspClient,
    documents: Mutex<HashMap<PathBuf, Document>>,
    last_used: std::sync::Mutex<Instant>,
    /// Connected `shared` sessions; the reaper leaves the server running while
    /// any is.
    sessions: AtomicUsize,
}

/// One document as the server last saw it.
struct Document {
    version: i32,
    text: String,
    /// `shared` sessions that opened it. The UI is never a holder: it resyncs
    /// from disk on every lookup, so closing under it loses nothing.
    holders: HashSet<u64>,
}

type ServerKey = (PathBuf, LanguageServer);

/// The servers this pod runs, keyed by workspace directory and language, for
/// the UI and, through `shared`, for the agent.
#[derive(Clone)]
pub struct CodeIntel {
    enabled: Vec<LanguageServer>,
    servers: Arc<Mutex<HashMap<ServerKey, Arc<Server>>>>,
    cancel: CancellationToken,
}

impl CodeIntel {
    /// `configured` is the Workspace's `languageServers`.
    pub fn new(configured: &[LanguageServer], cancel: CancellationToken) -> Self {
        Self {
            enabled: configured.to_vec(),
            servers: Arc::default(),
            cancel,
        }
    }

    pub(super) fn enabled(&self) -> &[LanguageServer] {
        &self.enabled
    }

    /// Start every configured server for `workspace_dir` in the background, so
    /// a slow indexer such as rust-analyzer is ready by the first lookup.
    pub fn warm(&self, workspace_dir: &str) {
        for &language in &self.enabled {
            let intel = self.clone();
            let root = PathBuf::from(workspace_dir);
            tokio::spawn(async move {
                if let Err(e) = intel.server(&root, language).await {
                    warn!(server = %language, error = %e, "Could not start language server");
                }
            });
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
        // JDT.LS holds a request until its import ends, so a busy server can
        // time out where rust-analyzer answers empty.
        if matches!(result, Err(LspError::Timeout(..))) && server.client.is_busy() {
            return Err(CodeIntelError::Indexing(language));
        }
        let found = locations(&result?, &root);
        if found.is_empty() && server.client.is_busy() {
            return Err(CodeIntelError::Indexing(language));
        }
        Ok(found)
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

    /// The running server whose root is `root` or a directory above it, started
    /// at `root` when there is none or the last one exited. An agent that has
    /// `cd`'d into a checkout names that checkout as its root, and still shares
    /// the server init started for the workspace directory.
    pub(super) async fn server(
        &self,
        root: &Path,
        language: LanguageServer,
    ) -> Result<Arc<Server>, LspError> {
        let key = (root.to_path_buf(), language);
        // Held across the start so two first lookups do not start two servers.
        let mut servers = self.servers.lock().await;
        let live = servers
            .iter()
            .filter(|(_, server)| server.client.is_alive())
            .map(|(key, _)| key);
        if let Some(server) = covering(live, root, language).and_then(|k| servers.get(k)) {
            server.touch();
            return Ok(Arc::clone(server));
        }
        let spec = server_spec(language);
        let started = Instant::now();
        // The options may read the checkout; a failed read starts the server
        // with its defaults.
        let options_root = root.to_path_buf();
        let options =
            tokio::task::spawn_blocking(move || (spec.initialization_options)(&options_root))
                .await
                .unwrap_or(Value::Null);
        let client =
            LspClient::start(spec.command, spec.args, root, options, START_TIMEOUT).await?;
        info!(server = %language, root = %root.display(), elapsed_ms = started.elapsed().as_millis() as u64, "Started language server");
        let server = Arc::new(Server {
            client,
            documents: Mutex::default(),
            last_used: std::sync::Mutex::new(Instant::now()),
            sessions: AtomicUsize::new(0),
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
    pub(super) fn touch(&self) {
        if let Ok(mut last_used) = self.last_used.lock() {
            *last_used = Instant::now();
        }
    }

    fn idle_for(&self) -> Duration {
        self.last_used
            .lock()
            .map_or(Duration::ZERO, |last_used| last_used.elapsed())
    }

    /// Keep the server from the reaper until the returned guard drops.
    pub(super) fn hold(self: &Arc<Self>) -> Held {
        self.sessions.fetch_add(1, Ordering::Relaxed);
        Held(Arc::clone(self))
    }

    /// Open `path` with `text`, or send the new text when it changed since the
    /// server last saw it. Returns the document's URI.
    async fn sync(&self, path: &Path, language_id: &str, text: String) -> Result<String, LspError> {
        let mut documents = self.documents.lock().await;
        self.sync_locked(&mut documents, path, language_id, text)
            .await
    }

    async fn sync_locked(
        &self,
        documents: &mut HashMap<PathBuf, Document>,
        path: &Path,
        language_id: &str,
        text: String,
    ) -> Result<String, LspError> {
        let uri = path_to_uri(path);
        match documents.get_mut(path) {
            Some(document) if document.text == text => {}
            Some(document) => {
                document.version += 1;
                self.client
                    .notify(
                        "textDocument/didChange",
                        json!({
                            "textDocument": { "uri": uri, "version": document.version },
                            "contentChanges": [{ "text": text }],
                        }),
                    )
                    .await?;
                document.text = text;
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
                documents.insert(
                    path.to_path_buf(),
                    Document {
                        version: 1,
                        text,
                        holders: HashSet::new(),
                    },
                );
            }
        }
        Ok(uri)
    }

    /// A session's `didOpen`.
    pub(super) async fn open(
        &self,
        session: u64,
        path: &Path,
        language_id: &str,
        text: String,
    ) -> Result<(), LspError> {
        let mut documents = self.documents.lock().await;
        self.sync_locked(&mut documents, path, language_id, text)
            .await?;
        if let Some(document) = documents.get_mut(path) {
            document.holders.insert(session);
        }
        Ok(())
    }

    /// A session's `didChange`, whole-text or ranged, applied to the text the
    /// server holds and sent on as one whole-text change.
    pub(super) async fn change(&self, path: &Path, changes: &[Value]) -> Result<(), LspError> {
        let mut documents = self.documents.lock().await;
        let Some(document) = documents.get(path) else {
            return Ok(());
        };
        let mut text = document.text.clone();
        for change in changes {
            let Some(new_text) = change.get("text").and_then(Value::as_str) else {
                continue;
            };
            match change.get("range") {
                Some(range) => apply_range(&mut text, range, new_text),
                None => new_text.clone_into(&mut text),
            }
        }
        // The language id only matters to an open, which a known path never needs.
        self.sync_locked(&mut documents, path, "", text).await?;
        Ok(())
    }

    /// A session's `didClose`. The server closes the document once no session
    /// holds it.
    pub(super) async fn close(&self, session: u64, path: &Path) -> Result<(), LspError> {
        let mut documents = self.documents.lock().await;
        let Some(document) = documents.get_mut(path) else {
            return Ok(());
        };
        document.holders.remove(&session);
        if document.holders.is_empty() {
            documents.remove(path);
            self.client
                .notify(
                    "textDocument/didClose",
                    json!({ "textDocument": { "uri": path_to_uri(path) } }),
                )
                .await?;
        }
        Ok(())
    }

    /// Close every document `session` alone held, when it disconnects.
    pub(super) async fn release(&self, session: u64) {
        let held: Vec<PathBuf> = self
            .documents
            .lock()
            .await
            .iter()
            .filter(|(_, document)| document.holders.contains(&session))
            .map(|(path, _)| path.clone())
            .collect();
        for path in held {
            if let Err(e) = self.close(session, &path).await {
                warn!(error = %e, "Could not close a document for a disconnected session");
            }
        }
    }
}

/// A connected `shared` session, counted in [`Server::sessions`].
pub(super) struct Held(Arc<Server>);

impl Drop for Held {
    fn drop(&mut self) {
        self.0.sessions.fetch_sub(1, Ordering::Relaxed);
        self.0.touch();
    }
}

/// The key, among `keys`, of the `language` server rooted closest above or at
/// `root`. Only JDT.LS imports every project below its root; rust-analyzer
/// looks for `Cargo.toml` one level down, so an ancestor's server would miss
/// a deeper crate, and the others match their exact root.
fn covering<'a>(
    keys: impl Iterator<Item = &'a ServerKey>,
    root: &Path,
    language: LanguageServer,
) -> Option<&'a ServerKey> {
    let shares_ancestor = language == LanguageServer::Java;
    keys.filter(|(server_root, server_language)| {
        *server_language == language
            && (root == server_root || (shares_ancestor && root.starts_with(server_root)))
    })
    .max_by_key(|(server_root, _)| server_root.components().count())
}

/// Replace an LSP `range` (UTF-16 positions) of `text` with `new_text`.
fn apply_range(text: &mut String, range: &Value, new_text: &str) {
    let offset = |key: &str| {
        let line = range.pointer(&format!("/{key}/line"))?.as_u64()?;
        let character = range.pointer(&format!("/{key}/character"))?.as_u64()?;
        Some(byte_offset(text, line, character))
    };
    if let (Some(start), Some(end)) = (offset("start"), offset("end")) {
        if start <= end {
            text.replace_range(start..end, new_text);
        }
    }
}

/// The byte offset of a 0-based line and UTF-16 character, clamped to the text.
fn byte_offset(text: &str, line: u64, character: u64) -> usize {
    let mut line_start = 0;
    for _ in 0..line {
        match text[line_start..].find('\n') {
            Some(newline) => line_start += newline + 1,
            None => return text.len(),
        }
    }
    let mut units = 0;
    for (index, ch) in text[line_start..].char_indices() {
        if units >= character || ch == '\n' {
            return line_start + index;
        }
        units += ch.len_utf16() as u64;
    }
    text.len()
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
        if live.idle_for() >= IDLE_TIMEOUT && live.sessions.load(Ordering::Relaxed) == 0 {
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
    fn configured_servers_are_matched_by_extension() {
        let intel = CodeIntel::new(&[LanguageServer::Typescript], CancellationToken::new());
        assert_eq!(
            intel.server_for(Path::new("/w/a.tsx")),
            Some((LanguageServer::Typescript, "typescriptreact"))
        );
        assert!(intel.server_for(Path::new("/w/lib.rs")).is_none());
        assert!(intel.server_for(Path::new("/w/Makefile")).is_none());
    }

    #[test]
    fn a_root_inside_a_served_directory_shares_its_server() {
        let workspace = (PathBuf::from("/w"), LanguageServer::Java);
        let checkout = (PathBuf::from("/w/repo"), LanguageServer::Java);
        let rust = (PathBuf::from("/w"), LanguageServer::Rust);
        let keys = [workspace.clone(), checkout.clone(), rust.clone()];

        let at = |root: &str, language| covering(keys.iter(), Path::new(root), language);

        assert_eq!(at("/w/repo/sub", LanguageServer::Java), Some(&checkout));
        assert_eq!(at("/w/other", LanguageServer::Java), Some(&workspace));
        assert_eq!(at("/w2", LanguageServer::Java), None);
        assert_eq!(at("/w/repo", LanguageServer::Typescript), None);
        assert_eq!(at("/w/repo", LanguageServer::Rust), None);
        assert_eq!(at("/w", LanguageServer::Rust), Some(&rust));
    }

    #[test]
    fn ranged_changes_count_utf16_units_per_line() {
        let mut text = "fn a() {}\nlet s = \"🙂\"; x\n".to_string();
        // The emoji is two UTF-16 units, so `x` is at character 14 of line 1.
        let range = json!({ "start": { "line": 1, "character": 14 }, "end": { "line": 1, "character": 15 } });
        apply_range(&mut text, &range, "y");
        assert_eq!(text, "fn a() {}\nlet s = \"🙂\"; y\n");
        let past_end =
            json!({ "start": { "line": 9, "character": 0 }, "end": { "line": 9, "character": 0 } });
        apply_range(&mut text, &past_end, "z");
        assert_eq!(text, "fn a() {}\nlet s = \"🙂\"; y\nz");
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
