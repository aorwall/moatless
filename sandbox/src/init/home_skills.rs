//! Install the repo's skills into the home-level skill dirs for both agents.
//!
//! The agent's cwd is `workspace_dir`, but the task repo is cloned one level
//! down at `repo_dir`. Skills are discovered relative to cwd, so a repo that
//! ships `.claude/skills/` at its root would be invisible with cwd one level up.
//!
//! Rather than recreate the repo's config surface at the workspace root (which
//! pollutes cwd and pulls `CLAUDE.md`/`AGENTS.md` into the agent's project
//! context), we **copy** the repo's skills into the user-level skill directories
//! each agent already reads:
//!   - `<claude_config_dir>/skills/<name>`  (Claude)
//!   - `<codex_home_dir>/skills/<name>`     (Codex, `$CODEX_HOME/skills`)
//!
//! This mirrors how staged image skills are installed in [`super::dirs`]. Repo
//! skills join the staged skills already living in those dirs; an existing entry
//! of the same name (e.g. a staged skill) is left untouched, never overwritten.
//!
//! Only skills are copied — `CLAUDE.md`/`AGENTS.md` are intentionally left out.
//! The copies live in the home dir, outside `workspace_dir`/`repo_dir`, so the
//! workspace snapshot never sees them; they're regenerated on every init.

use std::path::Path;

use tracing::{debug, warn};

use super::dirs::copy_dir_contents;

/// Copy the repo's root `.claude/skills/<name>` subdirs into the home-level skill
/// directories for Claude (`<claude_config_dir>/skills`) and Codex
/// (`<codex_home_dir>/skills`). Returns the number of skills copied across both.
pub(crate) fn install_repo_skills_to_home(
    repo_dir: &Path,
    claude_config_dir: &Path,
    codex_home_dir: &Path,
) -> usize {
    let repo_skills = repo_dir.join(".claude").join("skills");
    if !repo_skills.is_dir() {
        return 0;
    }

    let mut copied = 0usize;
    copied += copy_repo_skills(&repo_skills, &claude_config_dir.join("skills"));
    copied += copy_repo_skills(&repo_skills, &codex_home_dir.join("skills"));
    copied
}

/// Copy each skill subdir under `repo_skills` into `dst_skills`, skipping any
/// name that already exists there. Returns the number of skills copied.
fn copy_repo_skills(repo_skills: &Path, dst_skills: &Path) -> usize {
    if let Err(e) = std::fs::create_dir_all(dst_skills) {
        warn!(path = %dst_skills.display(), error = %e, "Failed to create home skills dir");
        return 0;
    }

    let entries = match std::fs::read_dir(repo_skills) {
        Ok(e) => e,
        Err(e) => {
            warn!(path = %repo_skills.display(), error = %e, "Failed to read repo .claude/skills");
            return 0;
        }
    };

    let mut copied = 0usize;
    for entry in entries.flatten() {
        // `is_dir()` follows symlinks, so `.claude/skills/<name>` entries that
        // symlink into the repo's `.agents/skills` are copied through.
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name();
        let dst = dst_skills.join(&name);
        // symlink_metadata reports presence even for broken symlinks — never
        // clobber an existing (e.g. staged image) skill of the same name.
        if std::fs::symlink_metadata(&dst).is_ok() {
            continue;
        }
        if let Err(e) = std::fs::create_dir_all(&dst) {
            warn!(path = %dst.display(), error = %e, "Failed to create home skill target");
            continue;
        }
        match copy_dir_contents(&entry.path(), &dst) {
            Ok(_) => {
                debug!(skill = %name.to_string_lossy(), dst = %dst.display(), "Copied repo skill into home dir");
                copied += 1;
            }
            Err(e) => warn!(dst = %dst.display(), error = %e, "Failed to copy repo skill into home dir"),
        }
    }
    copied
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn copies_repo_skills_into_both_home_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(repo.join(".claude/skills/foo")).unwrap();
        fs::write(repo.join(".claude/skills/foo/SKILL.md"), "foo").unwrap();
        fs::create_dir_all(repo.join(".claude/skills/bar")).unwrap();
        // CLAUDE.md must NOT be copied anywhere.
        fs::write(repo.join("CLAUDE.md"), "instructions").unwrap();

        let claude_cfg = tmp.path().join("home/.claude");
        let codex_home = tmp.path().join("home/.codex");

        let count = install_repo_skills_to_home(&repo, &claude_cfg, &codex_home);
        // 2 skills × 2 home dirs (Claude + Codex) = 4.
        assert_eq!(count, 4);

        // Claude reads `<claude_config_dir>/skills`; Codex reads `<codex_home_dir>/skills`.
        for dst_skills in [claude_cfg.join("skills"), codex_home.join("skills")] {
            assert_eq!(
                fs::read_to_string(dst_skills.join("foo/SKILL.md")).unwrap(),
                "foo",
            );
            assert!(dst_skills.join("bar").is_dir());
        }
        // No docs are bridged.
        assert!(!tmp.path().join("home/CLAUDE.md").exists());
        assert!(!tmp.path().join("home/AGENTS.md").exists());
    }

    #[test]
    fn follows_symlinked_skill_dirs() {
        // This repo authors skills under `.agents/skills` and exposes them via
        // `.claude/skills/<name>` symlinks — the copy must resolve through them.
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(repo.join(".agents/skills/foo")).unwrap();
        fs::write(repo.join(".agents/skills/foo/SKILL.md"), "real").unwrap();
        fs::create_dir_all(repo.join(".claude/skills")).unwrap();
        std::os::unix::fs::symlink("../../.agents/skills/foo", repo.join(".claude/skills/foo"))
            .unwrap();

        let claude_cfg = tmp.path().join("home/.claude");
        let codex_home = tmp.path().join("home/.codex");

        let count = install_repo_skills_to_home(&repo, &claude_cfg, &codex_home);
        assert_eq!(count, 2);
        assert_eq!(
            fs::read_to_string(claude_cfg.join("skills/foo/SKILL.md")).unwrap(),
            "real",
        );
    }

    #[test]
    fn does_not_overwrite_existing_home_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(repo.join(".claude/skills/foo")).unwrap();
        fs::write(repo.join(".claude/skills/foo/SKILL.md"), "repo").unwrap();

        let claude_cfg = tmp.path().join("home/.claude");
        let codex_home = tmp.path().join("home/.codex");
        // A staged skill of the same name already lives in the Claude home dir.
        fs::create_dir_all(claude_cfg.join("skills/foo")).unwrap();
        fs::write(claude_cfg.join("skills/foo/SKILL.md"), "staged").unwrap();

        let count = install_repo_skills_to_home(&repo, &claude_cfg, &codex_home);
        // Claude side skipped (already present); Codex side copied → 1.
        assert_eq!(count, 1);
        assert_eq!(
            fs::read_to_string(claude_cfg.join("skills/foo/SKILL.md")).unwrap(),
            "staged",
        );
        assert_eq!(
            fs::read_to_string(codex_home.join("skills/foo/SKILL.md")).unwrap(),
            "repo",
        );
    }

    #[test]
    fn noop_without_repo_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let claude_cfg = tmp.path().join("home/.claude");
        let codex_home = tmp.path().join("home/.codex");
        assert_eq!(install_repo_skills_to_home(&repo, &claude_cfg, &codex_home), 0);
    }

    #[test]
    fn is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(repo.join(".claude/skills/foo")).unwrap();
        let claude_cfg = tmp.path().join("home/.claude");
        let codex_home = tmp.path().join("home/.codex");

        let first = install_repo_skills_to_home(&repo, &claude_cfg, &codex_home);
        let second = install_repo_skills_to_home(&repo, &claude_cfg, &codex_home);
        assert_eq!(first, 2);
        assert_eq!(second, 0);
    }
}
