//! Install the repo's skills into the home-level skill dirs for both agents.
//!
//! The agent's cwd is `workspace_dir`, but the task repo is cloned one level
//! down at `repo_dir`. Skills are discovered relative to cwd, so a repo that
//! ships `.claude/skills/` (or `.agents/skills/`) at its root would be invisible
//! with cwd one level up.
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

/// Repo-relative skill roots, in precedence order. Both are scanned and merged
/// so a repo that authors its skills under `.agents/skills` is picked up whether
/// or not it also exposes them via `.claude/skills` symlinks. Mirrors
/// `WORKSPACE_SKILL_PREFIXES` in the backend's skill fetcher.
const REPO_SKILL_ROOTS: [&str; 2] = [".claude/skills", ".agents/skills"];

/// Copy the repo's `<root>/<name>` skill subdirs — for each root in
/// [`REPO_SKILL_ROOTS`] — into the home-level skill directories for Claude
/// (`<claude_config_dir>/skills`) and Codex (`<codex_home_dir>/skills`).
/// Returns the number of skills copied across both.
pub(crate) fn install_repo_skills_to_home(
    repo_dir: &Path,
    claude_config_dir: &Path,
    codex_home_dir: &Path,
) -> usize {
    let repo_skills: Vec<_> = REPO_SKILL_ROOTS
        .iter()
        .map(|root| repo_dir.join(root))
        .filter(|src| src.is_dir())
        .collect();
    if repo_skills.is_empty() {
        return 0;
    }

    let mut copied = 0usize;
    for dst in [claude_config_dir.join("skills"), codex_home_dir.join("skills")] {
        // Roots are walked in precedence order per destination, and
        // `copy_repo_skills` skips names already present — so a skill exposed
        // under both roots (typically a `.claude/skills/foo` symlink into
        // `.agents/skills/foo`) is copied once, from `.claude/skills`.
        for src in &repo_skills {
            copied += copy_repo_skills(src, &dst);
        }
    }
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
            Err(e) => {
                warn!(dst = %dst.display(), error = %e, "Failed to copy repo skill into home dir");
            }
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
    fn copies_agents_skills_without_any_claude_skills_dir() {
        // A repo that authors its skills under `.agents/skills` only, with no
        // `.claude/skills` directory at all.
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(repo.join(".agents/skills/foo")).unwrap();
        fs::write(repo.join(".agents/skills/foo/SKILL.md"), "agents").unwrap();

        let claude_cfg = tmp.path().join("home/.claude");
        let codex_home = tmp.path().join("home/.codex");

        let count = install_repo_skills_to_home(&repo, &claude_cfg, &codex_home);
        assert_eq!(count, 2);
        for dst_skills in [claude_cfg.join("skills"), codex_home.join("skills")] {
            assert_eq!(
                fs::read_to_string(dst_skills.join("foo/SKILL.md")).unwrap(),
                "agents",
            );
        }
    }

    #[test]
    fn merges_both_roots_with_claude_skills_winning() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        // `shared` lives under both roots; `only_agents` under `.agents` alone.
        fs::create_dir_all(repo.join(".claude/skills/shared")).unwrap();
        fs::write(repo.join(".claude/skills/shared/SKILL.md"), "claude").unwrap();
        fs::create_dir_all(repo.join(".agents/skills/shared")).unwrap();
        fs::write(repo.join(".agents/skills/shared/SKILL.md"), "agents").unwrap();
        fs::create_dir_all(repo.join(".agents/skills/only_agents")).unwrap();
        fs::write(repo.join(".agents/skills/only_agents/SKILL.md"), "agents").unwrap();

        let claude_cfg = tmp.path().join("home/.claude");
        let codex_home = tmp.path().join("home/.codex");

        let count = install_repo_skills_to_home(&repo, &claude_cfg, &codex_home);
        // 2 distinct skills × 2 home dirs — `shared` is not copied twice.
        assert_eq!(count, 4);
        for dst_skills in [claude_cfg.join("skills"), codex_home.join("skills")] {
            assert_eq!(
                fs::read_to_string(dst_skills.join("shared/SKILL.md")).unwrap(),
                "claude",
            );
            assert_eq!(
                fs::read_to_string(dst_skills.join("only_agents/SKILL.md")).unwrap(),
                "agents",
            );
        }
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
