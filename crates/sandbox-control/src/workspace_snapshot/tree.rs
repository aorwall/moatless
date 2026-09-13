//! Path handling shared by every stored-workspace format.
//!
//! Both backings answer the same three reads, and both reduce to a flat list of
//! [`SnapshotNode`]. Feeding one set of functions from either is what makes the
//! two produce byte-identical listings, so which format a task happens to hold
//! is invisible to a caller.

use std::collections::BTreeMap;

use moatless_ports::sandbox::SnapshotEntry;

/// Hops a symlink chain is followed before giving up.
///
/// A checkout commits symlinks, and neither backing stores content for a link,
/// so a read that did not follow would answer nothing for a file the person can
/// plainly see; the cap stops a cycle.
const SYMLINK_HOPS: usize = 8;

/// One stored file, reduced to what path handling needs.
#[derive(Debug, Clone)]
pub(super) struct SnapshotNode {
    /// Workspace-relative, `/`-separated.
    pub path: String,
    pub size: u64,
    /// The link target as stored, when this node is a symlink.
    pub symlink: Option<String>,
}

/// The path a read should actually fetch, following symlinks recorded in the
/// node set. `None` when the path names nothing, or a chain that does not
/// settle.
pub(super) fn resolve<'n>(nodes: &'n [SnapshotNode], path: &str) -> Option<&'n str> {
    let mut current = path.to_string();
    for _ in 0..SYMLINK_HOPS {
        let node = nodes.iter().find(|n| n.path == current)?;
        let Some(target) = node.symlink.as_deref() else {
            return Some(node.path.as_str());
        };
        current = if target.starts_with('/') {
            // An absolute link points outside the workspace's namespace; the
            // node set has nothing to say about it.
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

/// The prefix `path` names, as it appears in node paths. The root is the empty
/// string; everything else gains a trailing `/` so a match is on whole segments
/// and `src` does not select `src-gen/`.
pub(super) fn dir_prefix(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        String::new()
    } else {
        format!("{trimmed}/")
    }
}

/// `SKIP_DIRS` in `crates/sandbox/src/files/tree.rs`: directories a live walk
/// skips. Both back a repo's whole `.git`, so restore skips the clone.
///
/// Unfiltered, a listing would be mostly loose objects, pushing past every
/// caller's cap. A read is not filtered: only browsing has a cap to blow.
const UNBROWSABLE: &[&str] = &[".git", "node_modules"];

pub(super) fn browsable(path: &str) -> bool {
    !path
        .split('/')
        .any(|segment| UNBROWSABLE.contains(&segment))
}

/// Entries under `prefix`, sorted by path: non-recursive collapses each subtree
/// into one heading row; recursive lists every directory down.
///
/// Directories are synthesised from file paths even when the backing stores
/// real nodes, keeping formats' listings identical.
pub(super) fn listing_under(
    nodes: &[SnapshotNode],
    prefix: &str,
    recursive: bool,
) -> Vec<SnapshotEntry> {
    if recursive {
        let mut found: BTreeMap<String, SnapshotEntry> = BTreeMap::new();
        for node in nodes {
            if !node.path.starts_with(prefix) || !browsable(&node.path) {
                continue;
            }
            let mut cut = prefix.len();
            while let Some(slash) = node.path[cut..].find('/') {
                cut += slash + 1;
                let dir = node.path[..cut - 1].to_string();
                found.entry(dir.clone()).or_insert(SnapshotEntry {
                    path: dir,
                    is_dir: true,
                    size: 0,
                });
            }
            found.insert(
                node.path.clone(),
                SnapshotEntry {
                    path: node.path.clone(),
                    is_dir: false,
                    size: node.size,
                },
            );
        }
        return found.into_values().collect();
    }

    // Sorted and deduplicated: two files in one subdirectory must yield that
    // directory once.
    let mut children: BTreeMap<String, SnapshotEntry> = BTreeMap::new();
    for node in nodes {
        let Some(rest) = node.path.strip_prefix(prefix) else {
            continue;
        };
        if !browsable(&node.path) {
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
                    node.path.clone(),
                    SnapshotEntry {
                        path: node.path.clone(),
                        is_dir: false,
                        size: node.size,
                    },
                );
            }
        }
    }
    children.into_values().collect()
}

/// Files whose path contains `query`, capped at `limit` (0 means no cap).
pub(super) fn search_under(
    nodes: &[SnapshotNode],
    query: &str,
    case_sensitive: bool,
    limit: usize,
) -> Vec<SnapshotEntry> {
    let needle = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    let mut entries = Vec::new();
    for node in nodes.iter().filter(|n| browsable(&n.path)) {
        let hay = if case_sensitive {
            node.path.clone()
        } else {
            node.path.to_lowercase()
        };
        if !hay.contains(&needle) {
            continue;
        }
        entries.push(SnapshotEntry {
            path: node.path.clone(),
            is_dir: false,
            size: node.size,
        });
        if limit > 0 && entries.len() >= limit {
            break;
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> SnapshotNode {
        SnapshotNode {
            path: path.to_string(),
            size: 10,
            symlink: None,
        }
    }

    fn link(path: &str, target: &str) -> SnapshotNode {
        SnapshotNode {
            symlink: Some(target.to_string()),
            ..file(path)
        }
    }

    fn nodes() -> Vec<SnapshotNode> {
        vec![
            file("repo/AGENTS.md"),
            link("repo/CLAUDE.md", "AGENTS.md"),
            file("repo/src/lib.rs"),
            file("repo/src/deep/mod.rs"),
            file("repo/src-gen/out.rs"),
            file("top.txt"),
        ]
    }

    /// A checkout's `CLAUDE.md` is a symlink with no content of its own, so a
    /// read that did not follow it would answer nothing for a file the person
    /// can see in the tree.
    #[test]
    fn a_read_follows_a_symlink_to_the_file_that_holds_the_content() {
        let n = nodes();
        assert_eq!(resolve(&n, "repo/CLAUDE.md"), Some("repo/AGENTS.md"));
        assert_eq!(resolve(&n, "repo/AGENTS.md"), Some("repo/AGENTS.md"));
        assert_eq!(resolve(&n, "repo/nope.md"), None);
    }

    #[test]
    fn a_symlink_cycle_stops_rather_than_spinning() {
        let n = vec![link("a", "b"), link("b", "a")];
        assert_eq!(resolve(&n, "a"), None);
    }

    #[test]
    fn a_link_out_of_the_workspace_resolves_to_nothing() {
        let n = vec![link("a", "/etc/passwd"), link("b", "../../outside")];
        assert_eq!(resolve(&n, "a"), None);
        assert_eq!(resolve(&n, "b"), None);
    }

    /// A listing is on whole segments: `src` must not pull in `src-gen`.
    #[test]
    fn a_shallow_listing_names_each_subdirectory_once_and_matches_whole_segments() {
        let n = nodes();
        let entries = listing_under(&n, &dir_prefix("repo"), false);
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

        let src = listing_under(&n, &dir_prefix("repo/src"), false);
        let paths: Vec<&str> = src.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["repo/src/deep", "repo/src/lib.rs"]);
    }

    /// Every file under the prefix, and the directories they hang under: a
    /// client renders a tree and has no row to attach `deep/mod.rs` to
    /// otherwise.
    #[test]
    fn a_recursive_listing_names_the_directories_its_files_hang_under() {
        let n = nodes();
        let entries = listing_under(&n, &dir_prefix("repo/src"), true);
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["repo/src/deep", "repo/src/deep/mod.rs", "repo/src/lib.rs"]
        );
        assert!(entries[0].is_dir);
    }

    /// Both backings carry every repo's `.git` so a restore can skip the clone.
    /// A tree that showed it would be almost entirely loose objects.
    #[test]
    fn a_listing_leaves_out_what_a_live_walk_does_not_enter() {
        let n = vec![
            file("repo/src/lib.rs"),
            file("repo/.git/config"),
            file("repo/.git/objects/ab/cdef"),
            file("repo/node_modules/left-pad/index.js"),
        ];
        let deep = listing_under(&n, "", true);
        let paths: Vec<&str> = deep.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["repo", "repo/src", "repo/src/lib.rs"]);

        let shallow = listing_under(&n, &dir_prefix("repo"), false);
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

        let entries = listing_under(&nodes(), &dir_prefix("."), false);
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["repo", "top.txt"]);
    }

    #[test]
    fn a_search_caps_its_results_and_skips_what_a_listing_skips() {
        let n = vec![
            file("repo/src/lib.rs"),
            file("repo/src/LIB.md"),
            file("repo/.git/lib.rs"),
        ];
        let hits = search_under(&n, "lib", false, 0);
        let paths: Vec<&str> = hits.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["repo/src/lib.rs", "repo/src/LIB.md"]);

        assert_eq!(search_under(&n, "lib", true, 0).len(), 1);
        assert_eq!(search_under(&n, "lib", false, 1).len(), 1);
    }
}
