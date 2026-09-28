//! The plugin files a host's repository tarball holds, repacked without the
//! rest of the repository.
//!
//! Which files count mirrors how the sandbox resolves a plugin directory
//! (`plugin_skills::plugin_components`): a repository with a marketplace file
//! is that file plus each local entry's directory, and any other repository is
//! a plugin at its root.

use std::collections::BTreeSet;
use std::io::{self, Read};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use tar::{Archive, Builder, EntryType};
use tracing::warn;

use crate::skill::fetcher::{PLUGIN_MARKETPLACE_PATHS, PluginMarketplace, normalize_skills_path};
use moatless_workspace_types::plugin::marketplace_local_path;

/// The most file content a plugin keeps, so a small archive that inflates to
/// something huge fails instead of filling backend memory.
const MAX_PLUGIN_BYTES: u64 = 256 * 1024 * 1024;

/// The largest marketplace file read to decide which paths to keep.
const MAX_MARKETPLACE_BYTES: u64 = 1024 * 1024;

/// Repack `tarball` — a host archive with every entry under one top-level
/// directory — as a gzip tarball of only its plugin files, rooted at the
/// repository root.
pub(crate) fn plugin_files(tarball: &[u8]) -> io::Result<Vec<u8>> {
    let listing = list(tarball)?;
    let kept = kept_paths(&listing);

    let mut builder = Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    builder.follow_symlinks(false);
    let mut kept_bytes = 0u64;
    let mut archive = Archive::new(GzDecoder::new(tarball));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let Some(path) = repo_path(&entry)? else {
            continue;
        };
        if !kept.contains(&path) {
            continue;
        }
        let mut header = entry.header().clone();
        match header.entry_type() {
            EntryType::Symlink => {
                let target = entry.link_name()?.unwrap_or_default().into_owned();
                builder.append_link(&mut header, &path, target)?;
            }
            EntryType::Directory => builder.append_data(&mut header, &path, io::empty())?,
            _ => {
                kept_bytes += entry.size();
                if kept_bytes > MAX_PLUGIN_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("plugin files exceed {MAX_PLUGIN_BYTES} bytes"),
                    ));
                }
                builder.append_data(&mut header, &path, &mut entry)?;
            }
        }
    }
    builder.into_inner()?.finish()
}

/// What [`plugin_files`] decides from before it copies anything.
#[derive(Default)]
struct Listing {
    paths: Vec<String>,
    /// `(link path, target as written)` for every symlink.
    symlinks: Vec<(String, String)>,
    /// The first of [`PLUGIN_MARKETPLACE_PATHS`] present, with its content.
    marketplace: Option<(String, String)>,
}

fn list(tarball: &[u8]) -> io::Result<Listing> {
    let mut listing = Listing::default();
    let mut marketplaces: Vec<(String, String)> = Vec::new();
    let mut archive = Archive::new(GzDecoder::new(tarball));
    for entry in archive.entries()? {
        let entry = entry?;
        let Some(path) = repo_path(&entry)? else {
            continue;
        };
        if entry.header().entry_type() == EntryType::Symlink {
            let target = entry.link_name()?.unwrap_or_default();
            listing
                .symlinks
                .push((path.clone(), target.to_string_lossy().into_owned()));
        } else if PLUGIN_MARKETPLACE_PATHS.contains(&path.as_str()) {
            if entry.size() > MAX_MARKETPLACE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{path} exceeds {MAX_MARKETPLACE_BYTES} bytes"),
                ));
            }
            let mut raw = String::new();
            entry.take(MAX_MARKETPLACE_BYTES).read_to_string(&mut raw)?;
            marketplaces.push((path.clone(), raw));
        }
        listing.paths.push(path);
    }
    listing.marketplace = PLUGIN_MARKETPLACE_PATHS.iter().find_map(|wanted| {
        marketplaces
            .iter()
            .find(|(path, _)| path == wanted)
            .cloned()
    });
    Ok(listing)
}

/// An entry's path with the host's top-level directory stripped, or `None`
/// for the directory itself and for anything but a file, directory or symlink
/// (GitHub's `pax_global_header` among them).
fn repo_path<R: Read>(entry: &tar::Entry<'_, R>) -> io::Result<Option<String>> {
    let kind = entry.header().entry_type();
    if !(kind.is_file() || kind.is_dir() || kind.is_symlink()) {
        return Ok(None);
    }
    let path = entry.path()?;
    let path = path.to_string_lossy();
    let path = path.split_once('/').map_or("", |(_, rest)| rest);
    let path = path.trim_end_matches('/');
    Ok((!path.is_empty()).then(|| path.to_string()))
}

/// Every path to copy: the plugin roots, what their symlinks point at, and
/// the directories above both.
fn kept_paths(listing: &Listing) -> BTreeSet<String> {
    let mut roots = plugin_roots(listing);
    // A kept symlink into the rest of the repository keeps its target too, so
    // the sandbox does not unpack a dangling link.
    loop {
        let targets: Vec<String> = listing
            .symlinks
            .iter()
            .filter(|(link, _)| roots.iter().any(|root| under(link, root)))
            .filter_map(|(link, target)| resolve_link(link, target))
            .filter(|target| !roots.iter().any(|root| under(target, root)))
            .collect();
        if targets.is_empty() {
            break;
        }
        roots.extend(targets);
    }

    let mut kept = BTreeSet::new();
    for path in &listing.paths {
        if roots.iter().any(|root| under(path, root)) {
            kept.insert(path.clone());
        }
    }
    let ancestors: Vec<String> = kept
        .iter()
        .flat_map(|path| {
            path.match_indices('/')
                .map(|(at, _)| path[..at].to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    kept.extend(
        ancestors
            .into_iter()
            .filter(|dir| listing.paths.contains(dir)),
    );
    kept
}

/// Repository paths whose whole subtree is plugin content; `""` is the root.
fn plugin_roots(listing: &Listing) -> Vec<String> {
    let Some((marketplace_path, raw)) = &listing.marketplace else {
        return whole_repository(listing);
    };
    let marketplace: PluginMarketplace = match serde_json::from_str(raw) {
        Ok(marketplace) => marketplace,
        Err(e) => {
            warn!(marketplace = %marketplace_path, error = %e, "Invalid plugin marketplace — keeping the root plugin");
            return whole_repository(listing);
        }
    };
    let mut roots: Vec<String> = PLUGIN_MARKETPLACE_PATHS
        .iter()
        .map(ToString::to_string)
        .collect();
    for plugin in marketplace.plugins {
        let Some(path) = marketplace_local_path(&plugin.source, marketplace_path) else {
            continue;
        };
        match normalize_skills_path(&path) {
            Ok(root) if !root.is_empty() => roots.push(root),
            // An entry at the repository root keeps the root plugin.
            Ok(_) => return whole_repository(listing),
            Err(e) => warn!(path = %path, error = %e, "Rejected plugin marketplace path"),
        }
    }
    roots
}

/// Every top-level path but the host's own configuration (`.github/`,
/// `.gitignore` and the like), which no plugin reads.
fn whole_repository(listing: &Listing) -> Vec<String> {
    let top_level: BTreeSet<&str> = listing
        .paths
        .iter()
        .map(|path| path.split('/').next().unwrap_or(path))
        .filter(|name| !name.starts_with(".git") && *name != ".forgejo")
        .collect();
    top_level.into_iter().map(str::to_string).collect()
}

fn under(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// `target` read from the directory holding `link`, or `None` when it leaves
/// the repository.
fn resolve_link(link: &str, target: &str) -> Option<String> {
    if target.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = link.split('/').collect();
    parts.pop();
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            segment => parts.push(segment),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    enum Item<'a> {
        File(&'a str, &'a str),
        Link(&'a str, &'a str),
    }
    use Item::{File, Link};

    /// An archive shaped as GitHub serves one, `pax_global_header` included.
    fn host_archive(items: &[Item]) -> Vec<u8> {
        let mut builder = Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        let mut global = tar::Header::new_ustar();
        global.set_entry_type(EntryType::XGlobalHeader);
        global.set_size(0);
        builder
            .append_data(&mut global, "pax_global_header", io::empty())
            .unwrap();
        for item in items {
            let mut header = tar::Header::new_gnu();
            match item {
                File(path, contents) => {
                    header.set_size(contents.len() as u64);
                    header.set_mode(0o644);
                    builder
                        .append_data(&mut header, format!("acme-abc/{path}"), contents.as_bytes())
                        .unwrap();
                }
                Link(path, target) => {
                    header.set_entry_type(EntryType::Symlink);
                    header.set_size(0);
                    builder
                        .append_link(&mut header, format!("acme-abc/{path}"), target)
                        .unwrap();
                }
            }
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn paths(archive: &[u8]) -> Vec<String> {
        let mut archive = Archive::new(GzDecoder::new(archive));
        let mut paths: Vec<String> = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
            .collect();
        paths.sort();
        paths
    }

    const MARKETPLACE: &str = r#"{"plugins": [
        {"name": "general", "source": "./plugins/general"},
        {"name": "remote", "source": {"source": "github", "repo": "acme/other"}}
    ]}"#;

    #[test]
    fn a_marketplace_keeps_its_file_and_its_local_plugins_only() {
        let archive = host_archive(&[
            File(".claude-plugin/marketplace.json", MARKETPLACE),
            File("plugins/general/skills/review/SKILL.md", "review"),
            File(
                "plugins/generalist/README.md",
                "sibling with a shared prefix",
            ),
            File("app/src/main.ts", "application code"),
            File("README.md", "readme"),
        ]);

        let files = paths(&plugin_files(&archive).unwrap());

        assert_eq!(
            files,
            [
                ".claude-plugin/marketplace.json",
                "plugins/general/skills/review/SKILL.md",
            ]
        );
    }

    #[test]
    fn a_single_plugin_keeps_its_root_without_the_hosts_configuration() {
        let archive = host_archive(&[
            File(".claude-plugin/plugin.json", "{}"),
            File(".github/workflows/ci.yml", "ci"),
            File(".gitignore", "target"),
            File("scripts/hook.sh", "echo hi"),
            File("skills/review/SKILL.md", "review"),
        ]);

        let files = paths(&plugin_files(&archive).unwrap());

        assert_eq!(
            files,
            [
                ".claude-plugin/plugin.json",
                "scripts/hook.sh",
                "skills/review/SKILL.md",
            ]
        );
    }

    #[test]
    fn a_symlink_out_of_a_plugin_keeps_its_target() {
        let archive = host_archive(&[
            File(".claude-plugin/marketplace.json", MARKETPLACE),
            Link("plugins/general/skills/shared", "../../../shared/skill"),
            File("shared/skill/SKILL.md", "shared"),
            File("shared/other/SKILL.md", "not linked"),
            Link("plugins/general/escape", "../../../../etc/passwd"),
        ]);

        let files = paths(&plugin_files(&archive).unwrap());

        assert_eq!(
            files,
            [
                ".claude-plugin/marketplace.json",
                "plugins/general/escape",
                "plugins/general/skills/shared",
                "shared/skill/SKILL.md",
            ]
        );
    }

    #[test]
    fn an_invalid_marketplace_keeps_the_root_plugin() {
        let archive = host_archive(&[
            File(".claude-plugin/marketplace.json", "not json"),
            File("skills/review/SKILL.md", "review"),
        ]);

        let files = paths(&plugin_files(&archive).unwrap());

        assert_eq!(
            files,
            [".claude-plugin/marketplace.json", "skills/review/SKILL.md"]
        );
    }

    #[test]
    fn file_contents_and_modes_survive_the_repack() {
        let archive = host_archive(&[File("skills/review/SKILL.md", "review body")]);

        let repacked = plugin_files(&archive).unwrap();
        let mut archive = Archive::new(GzDecoder::new(repacked.as_slice()));
        let mut entry = archive.entries().unwrap().next().unwrap().unwrap();
        let mut body = String::new();
        entry.read_to_string(&mut body).unwrap();

        assert_eq!(body, "review body");
        assert_eq!(entry.header().mode().unwrap(), 0o644);
    }
}
