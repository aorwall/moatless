//! The stored format of a workspace mirror's manifest.
//!
//! The sandbox writes this object and the backend reads it, so the definition
//! lives here rather than in either — the same reason
//! [`crate::is_safe_workspace_path`] does. `manifest.json.gz` under a task's
//! prefix is the mirror's commit point: it names every file the mirror holds,
//! and each non-symlink entry has one object beside it.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};

/// The only manifest version this build writes or reads. A manifest carrying
/// any other version is declined by both sides.
pub const MANIFEST_VERSION: u32 = 1;

/// One mirrored file, as the manifest records it.
///
/// `size` and `mtime_ns` are the change signal [`Entry::matches`] compares, not
/// a checksum: they are what a walk can read without opening the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Workspace-relative path, `/`-separated.
    pub path: String,
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch, or `0` when the
    /// filesystem would not say. A symlink records `0`: a restore cannot
    /// reproduce a link's own mtime, so recording it would make every
    /// post-restore sync ship the manifest again.
    pub mtime_ns: u64,
    /// Permission bits, so an executable restores executable.
    pub mode: u32,
    /// Link target when this entry is a symlink. A symlink has no object in S3 —
    /// the restore recreates it from this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symlink: Option<String>,
}

impl Entry {
    /// Whether the two records describe the same stored bytes.
    pub fn matches(&self, other: &Self) -> bool {
        self.size == other.size
            && self.mtime_ns == other.mtime_ns
            && self.mode == other.mode
            && self.symlink == other.symlink
    }

    pub fn is_symlink(&self) -> bool {
        self.symlink.is_some()
    }
}

/// Everything the mirror holds for one workspace, sorted by path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Format version. A manifest whose version a build does not know is
    /// declined; see [`MANIFEST_VERSION`].
    pub version: u32,
    pub entries: Vec<Entry>,
}

impl Manifest {
    pub fn new(entries: Vec<Entry>) -> Self {
        Self {
            version: MANIFEST_VERSION,
            entries,
        }
    }

    /// Whether the workspace holds the primary repo's `.git`, which is what
    /// makes the mirror a complete set of repos rather than a working tree.
    pub fn carries_git(&self, primary_prefix: &str) -> bool {
        let marker = if primary_prefix.is_empty() {
            ".git/".to_string()
        } else {
            format!("{primary_prefix}/.git/")
        };
        self.entries.iter().any(|e| e.path.starts_with(&marker))
    }

    pub fn to_gz_json(&self) -> std::io::Result<Vec<u8>> {
        let json = serde_json::to_vec(self)?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&json)?;
        encoder.finish()
    }

    pub fn from_gz_json(bytes: &[u8]) -> std::io::Result<Self> {
        let mut json = Vec::new();
        GzDecoder::new(bytes).read_to_end(&mut json)?;
        serde_json::from_slice(&json).map_err(std::io::Error::other)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixtures must succeed"
)]
mod tests {
    use super::*;

    fn entry(path: &str) -> Entry {
        Entry {
            path: path.to_string(),
            size: 1,
            mtime_ns: 2,
            mode: 0o644,
            symlink: None,
        }
    }

    #[test]
    fn a_manifest_survives_the_gz_json_round_trip() {
        let manifest = Manifest::new(vec![entry("a.txt"), entry("dir/b.txt")]);
        let restored = Manifest::from_gz_json(&manifest.to_gz_json().unwrap()).unwrap();
        assert_eq!(restored, manifest);
        assert_eq!(restored.version, MANIFEST_VERSION);
    }

    /// The shape test the restore branches on: a repo's `.git` under the
    /// primary prefix means a complete set of repos, a bare working tree does
    /// not.
    #[test]
    fn carries_git_looks_under_the_primary_prefix() {
        let nested = Manifest::new(vec![entry("moatless/.git/HEAD")]);
        assert!(nested.carries_git("moatless"));
        assert!(!nested.carries_git(""));

        let root = Manifest::new(vec![entry(".git/HEAD")]);
        assert!(root.carries_git(""));
        assert!(!root.carries_git("moatless"));

        // A file merely named like the marker is not the marker.
        let decoy = Manifest::new(vec![entry("moatless/.gitignore")]);
        assert!(!decoy.carries_git("moatless"));
    }
}
