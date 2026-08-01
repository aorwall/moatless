//! Agent-instruction compatibility shims.
//!
//! `AGENTS.md` is the canonical instruction file for this repo. The sandbox
//! links `CLAUDE.md` next to authored `AGENTS.md` files so Claude reads the same
//! guidance. Legacy repos that only have `CLAUDE.md` still get an `AGENTS.md`
//! link so Codex can read them (Codex stacks AGENTS.md from repo root to cwd and
//! reads the AGENTS.md in the directory of files it edits; see
//! <https://developers.openai.com/codex/guides/agents-md>).
//!
//! The bridge uses **symlinks** with relative targets so the source of truth
//! stays in the repo and edits don't drift. Existing files or symlinks at the
//! destination are skipped — never overwritten — so authored instruction files
//! in the repo are preserved.
//!
//! Skills are **not** mirrored here. Skills are discovered relative to the
//! agent's cwd (the workspace), not throughout the repo tree, so
//! [`super::home_skills`] copies them into the home-level skill dirs for both
//! Claude (`<claude_config_dir>/skills`) and Codex (`<codex_home_dir>/skills`).
//!
//! Runs unconditionally in `run_core_init()` after the repo is on disk.
//! Agent type is decided per-message, so we always pre-stage. Operations
//! are cheap, idempotent, and harmless when Claude is the active agent.

use std::os::unix::fs::symlink;
use std::path::Path;

use tracing::{debug, warn};

const PRUNED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    ".next",
    ".turbo",
    ".cache",
];

/// Walk `workspace_dir` and create missing compatibility symlinks between
/// `AGENTS.md` and `CLAUDE.md`. Returns the number of symlinks created.
pub(crate) fn link_agent_instruction_files(workspace_dir: &Path) -> usize {
    if !workspace_dir.is_dir() {
        return 0;
    }

    let mut created = 0usize;
    walk_with_prune(workspace_dir, &mut |dir| {
        let agents_md = dir.join("AGENTS.md");
        let claude_md = dir.join("CLAUDE.md");

        if agents_md.is_file() && std::fs::symlink_metadata(&claude_md).is_err() {
            match symlink("AGENTS.md", &claude_md) {
                Ok(()) => {
                    debug!(path = %claude_md.display(), "Created CLAUDE.md -> AGENTS.md symlink");
                    created += 1;
                }
                Err(e) => warn!(
                    path = %claude_md.display(),
                    error = %e,
                    "Failed to create CLAUDE.md symlink",
                ),
            }
            return;
        }

        if claude_md.is_file() && std::fs::symlink_metadata(&agents_md).is_err() {
            match symlink("CLAUDE.md", &agents_md) {
                Ok(()) => {
                    debug!(path = %agents_md.display(), "Created AGENTS.md -> CLAUDE.md symlink");
                    created += 1;
                }
                Err(e) => warn!(
                    path = %agents_md.display(),
                    error = %e,
                    "Failed to create AGENTS.md symlink",
                ),
            }
        }
    });

    created
}

fn walk_with_prune<F>(root: &Path, visit: &mut F)
where
    F: FnMut(&Path),
{
    visit(root);

    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(e) => {
            warn!(path = %root.display(), error = %e, "Failed to read directory");
            return;
        }
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        if PRUNED_DIRS.iter().any(|p| name == *p) {
            continue;
        }
        walk_with_prune(&entry.path(), visit);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn links_claude_md_to_authored_agents_md_at_root_and_nested() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("AGENTS.md"), "root").unwrap();
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/AGENTS.md"), "nested").unwrap();

        let count = link_agent_instruction_files(root);

        assert_eq!(count, 2);
        let root_link = root.join("CLAUDE.md");
        let nested_link = root.join("nested/CLAUDE.md");
        assert_eq!(fs::read_link(&root_link).unwrap(), Path::new("AGENTS.md"));
        assert_eq!(fs::read_link(&nested_link).unwrap(), Path::new("AGENTS.md"));
        assert_eq!(fs::read_to_string(&root_link).unwrap(), "root");
        assert_eq!(fs::read_to_string(&nested_link).unwrap(), "nested");
    }

    #[test]
    fn links_agents_md_for_legacy_claude_only_repos() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("CLAUDE.md"), "legacy").unwrap();

        let count = link_agent_instruction_files(root);

        assert_eq!(count, 1);
        let agents_link = root.join("AGENTS.md");
        assert_eq!(fs::read_link(&agents_link).unwrap(), Path::new("CLAUDE.md"));
        assert_eq!(fs::read_to_string(&agents_link).unwrap(), "legacy");
    }

    #[test]
    fn skips_pruned_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for pruned in &["node_modules", "target", ".git"] {
            fs::create_dir_all(root.join(pruned)).unwrap();
            fs::write(root.join(pruned).join("CLAUDE.md"), "skip").unwrap();
        }

        let count = link_agent_instruction_files(root);

        assert_eq!(count, 0);
        for pruned in &["node_modules", "target", ".git"] {
            assert!(!root.join(pruned).join("AGENTS.md").exists());
        }
    }

    #[test]
    fn does_not_overwrite_existing_claude_md() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("AGENTS.md"), "agents").unwrap();
        fs::write(root.join("CLAUDE.md"), "preexisting").unwrap();

        let count = link_agent_instruction_files(root);

        assert_eq!(count, 0);
        assert_eq!(
            fs::read_to_string(root.join("CLAUDE.md")).unwrap(),
            "preexisting"
        );
        assert!(fs::read_link(root.join("CLAUDE.md")).is_err());
    }

    #[test]
    fn instruction_linking_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("AGENTS.md"), "agents").unwrap();

        let first = link_agent_instruction_files(root);
        let second = link_agent_instruction_files(root);
        assert_eq!(first, 1);
        assert_eq!(second, 0);
    }
}
