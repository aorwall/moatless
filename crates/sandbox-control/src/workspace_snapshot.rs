//! Reading a stopped task's workspace out of the S3 file mirror.
//!
//! The mirror (`crates/sandbox/src/init/workspace_mirror.rs`) stores one object per
//! file plus a `manifest.json.gz` naming them all, which is what makes a single
//! file reachable without unpacking anything — and therefore without a sandbox.
//! This is the read half.
//!
//! **Only the mirror.** A task that only ever wrote `workspace.tar.gz` has no
//! manifest and answers [`SnapshotError::NoSnapshot`]: serving one file out of a
//! tar means fetching and scanning the whole archive per request, which is the
//! cost the mirror exists to remove.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_ports::sandbox::{
    SnapshotEntry, SnapshotError, SnapshotFile, SnapshotListing, WorkspaceSnapshotSource,
};
use moatless_sandbox_runtime::storage::{manifest_key, workspace_object_key};
use sandbox_proto::is_safe_workspace_path;
use sandbox_proto::workspace_manifest::{MANIFEST_VERSION, Manifest};
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// How long a fetched manifest is reused.
///
/// A stopped sandbox's mirror does not change, but a task can be started and
/// sync again while a manifest is held, so this bounds how long a listing can
/// describe the previous generation. Short enough that a restarted task's first
/// listing is current, long enough that walking a tree costs one fetch.
const MANIFEST_TTL: Duration = Duration::from_secs(30);

/// Manifests held at once. A manifest is a few hundred KB for a large repo, so
/// this bounds the cache at tens of MB rather than at however many tasks were
/// browsed.
const MANIFEST_CACHE_CAP: usize = 32;

/// Hops a symlink chain is followed before giving up.
///
/// A checkout commits symlinks (`CLAUDE.md` -> `AGENTS.md` throughout this
/// repository), and the mirror stores a link as a manifest entry with no object,
/// so a read that did not follow would answer nothing for a file the person can
/// plainly see. The cap is what stops a cycle.
const SYMLINK_HOPS: usize = 8;

struct Cached {
    manifest: Arc<Manifest>,
    synced_at: DateTime<Utc>,
    fetched: Instant,
}

/// [`WorkspaceSnapshotSource`] over the S3 workspace mirror.
pub struct S3WorkspaceSnapshots {
    s3: Arc<moatless_s3::S3Storage>,
    bucket: String,
    prefix: String,
    cache: Mutex<HashMap<String, Cached>>,
}

impl S3WorkspaceSnapshots {
    pub fn new(s3: Arc<moatless_s3::S3Storage>, bucket: String, prefix: String) -> Self {
        Self {
            s3,
            bucket,
            prefix,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The task's manifest and the time it was stored, from cache when fresh.
    async fn manifest(
        &self,
        task_id: &str,
    ) -> Result<(Arc<Manifest>, DateTime<Utc>), SnapshotError> {
        {
            let cache = self.cache.lock().await;
            if let Some(hit) = cache.get(task_id) {
                if hit.fetched.elapsed() < MANIFEST_TTL {
                    return Ok((Arc::clone(&hit.manifest), hit.synced_at));
                }
            }
        }

        let key = manifest_key(&self.prefix, task_id);
        let synced_at =
            match moatless_s3::head_object_modified_at(&self.s3.client, &self.bucket, &key).await {
                Ok(Some(at)) => at,
                Ok(None) => return Err(SnapshotError::NoSnapshot),
                Err(e) => return Err(SnapshotError::Unavailable(e.to_string())),
            };

        let bytes = match moatless_s3::get_object_opt(&self.s3.client, &self.bucket, &key).await {
            Ok(Some(bytes)) => bytes,
            // The HEAD above saw it, so a miss here is a delete racing this read
            // rather than a task without a mirror.
            Ok(None) => return Err(SnapshotError::NoSnapshot),
            Err(e) => return Err(SnapshotError::Unavailable(e.to_string())),
        };

        let manifest = Manifest::from_gz_json(&bytes)
            .map_err(|e| SnapshotError::Unavailable(format!("manifest is unreadable: {e}")))?;
        if manifest.version != MANIFEST_VERSION {
            warn!(
                task_id = %task_id,
                version = manifest.version,
                "Stored manifest is a version this build cannot read"
            );
            return Err(SnapshotError::NoSnapshot);
        }

        debug!(task_id = %task_id, files = manifest.entries.len(), "Loaded a workspace snapshot manifest");
        let manifest = Arc::new(manifest);
        let mut cache = self.cache.lock().await;
        cache.retain(|_, held| held.fetched.elapsed() < MANIFEST_TTL);
        if cache.len() >= MANIFEST_CACHE_CAP {
            if let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, held)| held.fetched)
                .map(|(id, _)| id.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(
            task_id.to_string(),
            Cached {
                manifest: Arc::clone(&manifest),
                synced_at,
                fetched: Instant::now(),
            },
        );
        Ok((manifest, synced_at))
    }
}

/// The path a read should actually fetch, following symlinks recorded in the
/// manifest. `None` when the path names nothing, or a chain that does not settle.
fn resolve<'m>(manifest: &'m Manifest, path: &str) -> Option<&'m str> {
    let mut current = path.to_string();
    for _ in 0..SYMLINK_HOPS {
        let entry = manifest.entries.iter().find(|e| e.path == current)?;
        let Some(target) = entry.symlink.as_deref() else {
            return Some(entry.path.as_str());
        };
        current = if target.starts_with('/') {
            // An absolute link points outside the mirror's namespace; the
            // manifest has nothing to say about it.
            return None;
        } else {
            let parent = current.rsplit_once('/').map_or("", |(dir, _)| dir);
            normalize(parent, target)?
        };
    }
    None
}

/// Join a relative link target onto its directory and resolve `.` and `..`,
/// refusing anything that climbs above the workspace root.
fn normalize(parent: &str, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = if parent.is_empty() {
        Vec::new()
    } else {
        parent.split('/').collect()
    };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// The prefix `path` names, as it appears in manifest entries. The root is the
/// empty string; everything else gains a trailing `/` so a match is on whole
/// segments and `src` does not select `src-gen/`.
fn dir_prefix(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        String::new()
    } else {
        format!("{trimmed}/")
    }
}

/// Directories a live walk does not enter — `SKIP_DIRS` in
/// `crates/sandbox/src/files/tree.rs`. The mirror stores each repo's whole `.git` so a
/// restore can skip the clone, so a listing that did not apply this would be
/// mostly loose objects and would push the real tree past every caller's cap.
/// A read is not filtered: only browsing has a cap to blow.
const UNBROWSABLE: &[&str] = &[".git", "node_modules"];

fn browsable(path: &str) -> bool {
    !path
        .split('/')
        .any(|segment| UNBROWSABLE.contains(&segment))
}

/// Entries under `prefix`, sorted by path. Non-recursive collapses each subtree
/// into the one directory that heads it, which is what a file tree renders;
/// recursive names every directory on the way down as a row of its own, because
/// the mirror stores only files and a tree has nowhere to hang a child whose
/// parent it has no row for.
fn listing_under(manifest: &Manifest, prefix: &str, recursive: bool) -> Vec<SnapshotEntry> {
    if recursive {
        let mut found: BTreeMap<String, SnapshotEntry> = BTreeMap::new();
        for entry in &manifest.entries {
            if !entry.path.starts_with(prefix) || !browsable(&entry.path) {
                continue;
            }
            let mut cut = prefix.len();
            while let Some(slash) = entry.path[cut..].find('/') {
                cut += slash + 1;
                let dir = entry.path[..cut - 1].to_string();
                found.entry(dir.clone()).or_insert(SnapshotEntry {
                    path: dir,
                    is_dir: true,
                    size: 0,
                });
            }
            found.insert(
                entry.path.clone(),
                SnapshotEntry {
                    path: entry.path.clone(),
                    is_dir: false,
                    size: entry.size,
                },
            );
        }
        return found.into_values().collect();
    }

    // Sorted and deduplicated: two files in one subdirectory must yield that
    // directory once.
    let mut children: BTreeMap<String, SnapshotEntry> = BTreeMap::new();
    for entry in &manifest.entries {
        let Some(rest) = entry.path.strip_prefix(prefix) else {
            continue;
        };
        if !browsable(&entry.path) {
            continue;
        }
        match rest.split_once('/') {
            Some((dir, _)) if !dir.is_empty() => {
                let path = format!("{prefix}{dir}");
                children.entry(path.clone()).or_insert(SnapshotEntry {
                    path,
                    is_dir: true,
                    size: 0,
                });
            }
            _ => {
                children.insert(
                    entry.path.clone(),
                    SnapshotEntry {
                        path: entry.path.clone(),
                        is_dir: false,
                        size: entry.size,
                    },
                );
            }
        }
    }
    children.into_values().collect()
}

#[async_trait]
impl WorkspaceSnapshotSource for S3WorkspaceSnapshots {
    async fn read(&self, task_id: &str, path: &str) -> Result<SnapshotFile, SnapshotError> {
        let (manifest, synced_at) = self.manifest(task_id).await?;
        let stored =
            resolve(&manifest, path.trim_start_matches('/')).ok_or(SnapshotError::NotFound)?;

        // The same rule the sandbox applied when it named the object. A path
        // that fails it cannot have been stored, so this is a miss and not a
        // refusal.
        if !is_safe_workspace_path(stored) {
            return Err(SnapshotError::NotFound);
        }

        let key = workspace_object_key(&self.prefix, task_id, stored);
        match moatless_s3::get_object_opt(&self.s3.client, &self.bucket, &key).await {
            Ok(Some(content)) => Ok(SnapshotFile { content, synced_at }),
            Ok(None) => Err(SnapshotError::NotFound),
            Err(e) => Err(SnapshotError::Unavailable(e.to_string())),
        }
    }

    async fn list(
        &self,
        task_id: &str,
        path: &str,
        recursive: bool,
    ) -> Result<SnapshotListing, SnapshotError> {
        let (manifest, synced_at) = self.manifest(task_id).await?;
        Ok(SnapshotListing {
            entries: listing_under(&manifest, &dir_prefix(path), recursive),
            synced_at,
        })
    }

    async fn search(
        &self,
        task_id: &str,
        query: &str,
        case_sensitive: bool,
        limit: usize,
    ) -> Result<SnapshotListing, SnapshotError> {
        let (manifest, synced_at) = self.manifest(task_id).await?;
        let needle = if case_sensitive {
            query.to_string()
        } else {
            query.to_lowercase()
        };
        let mut entries = Vec::new();
        for entry in manifest.entries.iter().filter(|e| browsable(&e.path)) {
            let hay = if case_sensitive {
                entry.path.clone()
            } else {
                entry.path.to_lowercase()
            };
            if !hay.contains(&needle) {
                continue;
            }
            entries.push(SnapshotEntry {
                path: entry.path.clone(),
                is_dir: false,
                size: entry.size,
            });
            if limit > 0 && entries.len() >= limit {
                break;
            }
        }
        Ok(SnapshotListing { entries, synced_at })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_proto::workspace_manifest::Entry;

    fn file(path: &str) -> Entry {
        Entry {
            path: path.to_string(),
            size: 10,
            mtime_ns: 1,
            mode: 0o644,
            symlink: None,
        }
    }

    fn link(path: &str, target: &str) -> Entry {
        Entry {
            symlink: Some(target.to_string()),
            ..file(path)
        }
    }

    fn manifest() -> Manifest {
        Manifest::new(vec![
            file("repo/AGENTS.md"),
            link("repo/CLAUDE.md", "AGENTS.md"),
            file("repo/src/lib.rs"),
            file("repo/src/deep/mod.rs"),
            file("repo/src-gen/out.rs"),
            file("top.txt"),
        ])
    }

    /// A checkout's `CLAUDE.md` is a symlink with no object of its own, so a
    /// read that did not follow it would answer nothing for a file the person
    /// can see in the tree.
    #[test]
    fn a_read_follows_a_symlink_to_the_file_that_has_an_object() {
        let m = manifest();
        assert_eq!(resolve(&m, "repo/CLAUDE.md"), Some("repo/AGENTS.md"));
        assert_eq!(resolve(&m, "repo/AGENTS.md"), Some("repo/AGENTS.md"));
        assert_eq!(resolve(&m, "repo/nope.md"), None);
    }

    #[test]
    fn a_symlink_cycle_stops_rather_than_spinning() {
        let m = Manifest::new(vec![link("a", "b"), link("b", "a")]);
        assert_eq!(resolve(&m, "a"), None);
    }

    #[test]
    fn a_link_out_of_the_workspace_resolves_to_nothing() {
        let m = Manifest::new(vec![link("a", "/etc/passwd"), link("b", "../../outside")]);
        assert_eq!(resolve(&m, "a"), None);
        assert_eq!(resolve(&m, "b"), None);
    }

    /// A listing is on whole segments: `src` must not pull in `src-gen`.
    #[test]
    fn a_shallow_listing_names_each_subdirectory_once_and_matches_whole_segments() {
        let m = manifest();
        let entries = listing_under(&m, &dir_prefix("repo"), false);
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "repo/AGENTS.md",
                "repo/CLAUDE.md",
                "repo/src",
                "repo/src-gen"
            ]
        );
        assert!(
            entries
                .iter()
                .find(|e| e.path == "repo/src")
                .unwrap()
                .is_dir
        );

        let src = listing_under(&m, &dir_prefix("repo/src"), false);
        let paths: Vec<&str> = src.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["repo/src/deep", "repo/src/lib.rs"]);
    }

    /// Every file under the prefix, and the directories they hang under: a
    /// client renders a tree and has no row to attach `deep/mod.rs` to
    /// otherwise.
    #[test]
    fn a_recursive_listing_names_the_directories_its_files_hang_under() {
        let m = manifest();
        let entries = listing_under(&m, &dir_prefix("repo/src"), true);
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["repo/src/deep", "repo/src/deep/mod.rs", "repo/src/lib.rs"]
        );
        assert!(entries[0].is_dir);
    }

    /// The mirror carries every repo's `.git` so a restore can skip the clone.
    /// A tree that showed it would be almost entirely loose objects.
    #[test]
    fn a_listing_leaves_out_what_a_live_walk_does_not_enter() {
        let m = Manifest::new(vec![
            file("repo/src/lib.rs"),
            file("repo/.git/config"),
            file("repo/.git/objects/ab/cdef"),
            file("repo/node_modules/left-pad/index.js"),
        ]);
        let deep = listing_under(&m, "", true);
        let paths: Vec<&str> = deep.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["repo", "repo/src", "repo/src/lib.rs"]);

        let shallow = listing_under(&m, &dir_prefix("repo"), false);
        let paths: Vec<&str> = shallow.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["repo/src"]);
    }

    #[test]
    fn the_root_is_named_by_an_empty_prefix_however_it_is_written() {
        assert_eq!(dir_prefix("."), "");
        assert_eq!(dir_prefix(""), "");
        assert_eq!(dir_prefix("/"), "");
        assert_eq!(dir_prefix("repo/"), "repo/");
        assert_eq!(dir_prefix("/repo"), "repo/");

        let entries = listing_under(&manifest(), &dir_prefix("."), false);
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["repo", "top.txt"]);
    }
}
