//! Codex-compatibility shims for Claude-flavored repos.
//!
//! When the agent type is Codex, the sandbox needs:
//!   - `AGENTS.md` next to every `CLAUDE.md` so Codex picks up the same
//!     instructions (Codex stacks AGENTS.md from repo root → cwd; see
//!     <https://developers.openai.com/codex/guides/agents-md>).
//!   - `.agents/skills/<name>` next to every `.claude/skills/<name>` so Codex
//!     discovers Claude-shipped skill bundles
//!     (<https://developers.openai.com/codex/skills>).
//!
//! Both layouts are mirrored as **symlinks** with relative targets so the
//! source of truth stays in the repo and edits don't drift. Existing files
//! or symlinks at the destination are skipped — never overwritten — so
//! authored `AGENTS.md` files in the repo are preserved.
//!
//! `.agents/skills` itself **must** be a real directory: openai/codex#11314
//! (closed "not planned") confirms a symlinked parent breaks skill discovery;
//! only individual subdirs may be symlinks.
//!
//! Runs unconditionally in `run_core_init()` after the repo is on disk.
//! Agent type is decided per-message, so we always pre-stage. Operations
//! are cheap, idempotent, and harmless when Claude is the active agent.

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

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

/// Walk `workspace_dir` and create `AGENTS.md` symlinks pointing at any
/// `CLAUDE.md` found. Returns the number of symlinks created.
pub(crate) fn mirror_claude_md_to_agents_md(workspace_dir: &Path) -> usize {
    if !workspace_dir.is_dir() {
        return 0;
    }

    let mut created = 0usize;
    walk_with_prune(workspace_dir, &mut |dir| {
        let claude_md = dir.join("CLAUDE.md");
        if !claude_md.is_file() {
            return;
        }
        let agents_md = dir.join("AGENTS.md");
        // symlink_metadata reports presence even for broken symlinks — skip
        // anything already at this path so authored AGENTS.md is preserved.
        if std::fs::symlink_metadata(&agents_md).is_ok() {
            return;
        }
        match symlink("CLAUDE.md", &agents_md) {
            Ok(()) => {
                debug!(path = %agents_md.display(), "Created AGENTS.md → CLAUDE.md symlink");
                created += 1;
            }
            Err(e) => warn!(
                path = %agents_md.display(),
                error = %e,
                "Failed to create AGENTS.md symlink",
            ),
        }
    });

    created
}

/// Walk `workspace_dir` and mirror every `.claude/skills/<name>` subdir as a
/// `.agents/skills/<name>` symlink. The `.agents/skills` parent is created
/// as a real directory because Codex bug #11314 prevents skill discovery
/// when that directory is itself a symlink. Returns the number of skill
/// subdirs symlinked.
pub(crate) fn mirror_claude_skills_to_agents_skills(workspace_dir: &Path) -> usize {
    if !workspace_dir.is_dir() {
        return 0;
    }

    let mut created = 0usize;
    walk_with_prune(workspace_dir, &mut |dir| {
        let claude_skills = dir.join(".claude").join("skills");
        if !claude_skills.is_dir() {
            return;
        }
        let agents_skills = dir.join(".agents").join("skills");

        match std::fs::symlink_metadata(&agents_skills) {
            Ok(meta) => {
                if !meta.is_dir() || meta.file_type().is_symlink() {
                    warn!(
                        path = %agents_skills.display(),
                        "Skipping skill mirror: .agents/skills exists but is not a real directory",
                    );
                    return;
                }
            }
            Err(_) => {
                if let Err(e) = std::fs::create_dir_all(&agents_skills) {
                    warn!(
                        path = %agents_skills.display(),
                        error = %e,
                        "Failed to create .agents/skills directory",
                    );
                    return;
                }
            }
        }

        let entries = match std::fs::read_dir(&claude_skills) {
            Ok(e) => e,
            Err(e) => {
                warn!(
                    path = %claude_skills.display(),
                    error = %e,
                    "Failed to read .claude/skills",
                );
                return;
            }
        };

        for entry in entries.flatten() {
            let name = entry.file_name();
            let dst = agents_skills.join(&name);
            if std::fs::symlink_metadata(&dst).is_ok() {
                continue;
            }
            // .agents/skills/<name> → ../../.claude/skills/<name>
            let rel_target = PathBuf::from("..")
                .join("..")
                .join(".claude")
                .join("skills")
                .join(&name);
            match symlink(&rel_target, &dst) {
                Ok(()) => {
                    debug!(
                        link = %dst.display(),
                        target = %rel_target.display(),
                        "Created .agents/skills symlink",
                    );
                    created += 1;
                }
                Err(e) => warn!(
                    path = %dst.display(),
                    error = %e,
                    "Failed to create skill symlink",
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
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn mirrors_claude_md_at_root_and_nested() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("CLAUDE.md"), "root").unwrap();
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/CLAUDE.md"), "nested").unwrap();

        let count = mirror_claude_md_to_agents_md(root);

        assert_eq!(count, 2);
        let root_link = root.join("AGENTS.md");
        let nested_link = root.join("nested/AGENTS.md");
        assert_eq!(fs::read_link(&root_link).unwrap(), Path::new("CLAUDE.md"));
        assert_eq!(fs::read_link(&nested_link).unwrap(), Path::new("CLAUDE.md"));
        assert_eq!(fs::read_to_string(&root_link).unwrap(), "root");
        assert_eq!(fs::read_to_string(&nested_link).unwrap(), "nested");
    }

    #[test]
    fn skips_pruned_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for pruned in &["node_modules", "target", ".git"] {
            fs::create_dir_all(root.join(pruned)).unwrap();
            fs::write(root.join(pruned).join("CLAUDE.md"), "skip").unwrap();
        }

        let count = mirror_claude_md_to_agents_md(root);

        assert_eq!(count, 0);
        for pruned in &["node_modules", "target", ".git"] {
            assert!(!root.join(pruned).join("AGENTS.md").exists());
        }
    }

    #[test]
    fn does_not_overwrite_existing_agents_md() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("CLAUDE.md"), "claude").unwrap();
        fs::write(root.join("AGENTS.md"), "preexisting").unwrap();

        let count = mirror_claude_md_to_agents_md(root);

        assert_eq!(count, 0);
        assert_eq!(
            fs::read_to_string(root.join("AGENTS.md")).unwrap(),
            "preexisting"
        );
        assert!(fs::read_link(root.join("AGENTS.md")).is_err());
    }

    #[test]
    fn agents_md_mirror_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("CLAUDE.md"), "claude").unwrap();

        let first = mirror_claude_md_to_agents_md(root);
        let second = mirror_claude_md_to_agents_md(root);
        assert_eq!(first, 1);
        assert_eq!(second, 0);
    }

    #[test]
    fn mirrors_claude_skills_subdirs_with_real_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let foo_dir = root.join(".claude/skills/foo");
        fs::create_dir_all(&foo_dir).unwrap();
        fs::write(foo_dir.join("SKILL.md"), "foo skill").unwrap();
        let bar_dir = root.join(".claude/skills/bar");
        fs::create_dir_all(&bar_dir).unwrap();
        fs::write(bar_dir.join("SKILL.md"), "bar skill").unwrap();

        let count = mirror_claude_skills_to_agents_skills(root);

        assert_eq!(count, 2);
        let agents_skills = root.join(".agents/skills");
        let meta = fs::symlink_metadata(&agents_skills).unwrap();
        assert!(meta.is_dir(), ".agents/skills must be a real directory");
        assert!(
            !meta.file_type().is_symlink(),
            ".agents/skills must not be a symlink",
        );

        let foo_link = agents_skills.join("foo");
        let bar_link = agents_skills.join("bar");
        assert_eq!(
            fs::read_link(&foo_link).unwrap(),
            Path::new("../../.claude/skills/foo"),
        );
        assert_eq!(
            fs::read_link(&bar_link).unwrap(),
            Path::new("../../.claude/skills/bar"),
        );
        assert_eq!(
            fs::read_to_string(foo_link.join("SKILL.md")).unwrap(),
            "foo skill",
        );
        assert_eq!(
            fs::read_to_string(bar_link.join("SKILL.md")).unwrap(),
            "bar skill",
        );
    }

    #[test]
    fn skill_mirror_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".claude/skills/foo")).unwrap();

        let first = mirror_claude_skills_to_agents_skills(root);
        let second = mirror_claude_skills_to_agents_skills(root);
        assert_eq!(first, 1);
        assert_eq!(second, 0);
    }

    #[test]
    fn refuses_to_use_symlinked_agents_skills_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".claude/skills/foo")).unwrap();
        let real_target = tmp.path().join("elsewhere");
        fs::create_dir_all(&real_target).unwrap();
        fs::create_dir_all(root.join(".agents")).unwrap();
        symlink(&real_target, root.join(".agents/skills")).unwrap();

        let count = mirror_claude_skills_to_agents_skills(root);

        assert_eq!(count, 0);
    }

    #[test]
    fn nested_claude_skills_directory_is_mirrored() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let nested_skill = root.join("apps/web/.claude/skills/foo");
        fs::create_dir_all(&nested_skill).unwrap();

        let count = mirror_claude_skills_to_agents_skills(root);

        assert_eq!(count, 1);
        let link = root.join("apps/web/.agents/skills/foo");
        assert_eq!(
            fs::read_link(&link).unwrap(),
            Path::new("../../.claude/skills/foo"),
        );
    }
}
