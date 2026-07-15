//! Bridge repo-root agent config up to the workspace root.
//!
//! The agent's cwd is `workspace_dir` (the persisted container dir), but the
//! task repo is cloned into `repo_dir = workspace_dir/<repo-name>`. Claude Code
//! and Codex discover project-level config **relative to cwd**, so a repo that
//! ships `.claude/skills/`, `CLAUDE.md`, or `AGENTS.md` at its root would be
//! invisible with cwd one level up.
//!
//! This module recreates that discovery surface at the workspace root as
//! **symlinks** back into the repo, so the source of truth stays in the repo and
//! edits never drift:
//!   - `workspace_dir/.claude/skills/<name>` → `repo_dir/.claude/skills/<name>`  (Claude)
//!   - `workspace_dir/.agents/skills/<name>` → `repo_dir/.claude/skills/<name>`  (Codex)
//!   - `workspace_dir/CLAUDE.md`             → `repo_dir/CLAUDE.md`
//!   - `workspace_dir/AGENTS.md`             → `repo_dir/AGENTS.md`
//!
//! Skills are covered for **both** agents here at the workspace level (the shared
//! cwd), rather than mirroring `.agents/skills` down inside the repo tree: skills
//! are discovered relative to cwd, and cwd is the workspace, so an in-repo mirror
//! would never be read.
//!
//! As with [`super::codex_compat`], each `skills` parent **must** be a real
//! directory (a symlinked parent breaks skill discovery); only the individual
//! skill subdirs are symlinks. Existing files/symlinks at any destination are
//! skipped — never overwritten.
//!
//! The links are cheap, idempotent, and regenerated on every init, so they never
//! need to be persisted. They live at the workspace root, *outside* `repo_dir`,
//! so the Phase 1 (repo-scoped) S3 snapshot excludes them for free; the Phase 2
//! whole-workspace sync must exclude `workspace_dir/.claude` + `AGENTS.md`.
//!
//! No-op when `repo_dir == workspace_dir` (no separate checkout) or when the repo
//! is not a subdirectory of the workspace.

use std::path::Path;

use tracing::{debug, warn};

/// Link the repo's root-level agent config into the workspace root so cwd-based
/// discovery finds it. Returns the number of symlinks created.
pub(crate) fn bridge_repo_config_to_workspace(workspace_dir: &Path, repo_dir: &Path) -> usize {
    if repo_dir == workspace_dir {
        return 0;
    }
    // We build relative symlink targets as `<repo-name>/...`, which is only valid
    // when the repo is a direct child of the workspace.
    let Some(repo_name) = repo_dir.strip_prefix(workspace_dir).ok().filter(|rel| {
        // Exactly one path component (`<repo-name>`), no nesting or `..`.
        rel.components().count() == 1
    }) else {
        debug!(
            workspace = %workspace_dir.display(),
            repo = %repo_dir.display(),
            "Skipping workspace bridge: repo is not a direct child of the workspace",
        );
        return 0;
    };
    let repo_name = repo_name.as_os_str();

    let mut created = 0usize;
    // Surface the repo's skills at the workspace root for both agents: Claude
    // reads `.claude/skills`, Codex reads `.agents/skills`. Both point at the
    // repo's `.claude/skills` (the single source of truth).
    created += bridge_skills(workspace_dir, repo_dir, repo_name, ".claude");
    created += bridge_skills(workspace_dir, repo_dir, repo_name, ".agents");
    // Root docs: `CLAUDE.md` (Claude) and `AGENTS.md` (Codex, itself a symlink
    // the codex mirror created next to the repo's CLAUDE.md).
    for doc in ["CLAUDE.md", "AGENTS.md"] {
        // symlink_metadata resolves broken symlinks too, so we only skip real
        // repo entries.
        if std::fs::symlink_metadata(repo_dir.join(doc)).is_err() {
            continue;
        }
        let dst = workspace_dir.join(doc);
        if std::fs::symlink_metadata(&dst).is_ok() {
            continue;
        }
        let target = Path::new(repo_name).join(doc);
        match std::os::unix::fs::symlink(&target, &dst) {
            Ok(()) => {
                debug!(link = %dst.display(), target = %target.display(), "Bridged repo doc to workspace root");
                created += 1;
            }
            Err(e) => warn!(link = %dst.display(), error = %e, "Failed to bridge repo doc"),
        }
    }

    created
}

/// Mirror the repo's root `.claude/skills/<name>` subdirs into
/// `workspace_dir/<agent_dir>/skills/<name>` as symlinks, with a real
/// `<agent_dir>/skills` parent directory. `agent_dir` is `.claude` (Claude) or
/// `.agents` (Codex); both point at the repo's `.claude/skills` source.
fn bridge_skills(
    workspace_dir: &Path,
    repo_dir: &Path,
    repo_name: &std::ffi::OsStr,
    agent_dir: &str,
) -> usize {
    let repo_skills = repo_dir.join(".claude").join("skills");
    if !repo_skills.is_dir() {
        return 0;
    }

    let dst_skills = workspace_dir.join(agent_dir).join("skills");
    match std::fs::symlink_metadata(&dst_skills) {
        Ok(meta) => {
            if !meta.is_dir() || meta.file_type().is_symlink() {
                warn!(
                    path = %dst_skills.display(),
                    "Skipping skill bridge: workspace skills dir exists but is not a real directory",
                );
                return 0;
            }
        }
        Err(_) => {
            if let Err(e) = std::fs::create_dir_all(&dst_skills) {
                warn!(path = %dst_skills.display(), error = %e, "Failed to create workspace skills dir");
                return 0;
            }
        }
    }

    let entries = match std::fs::read_dir(&repo_skills) {
        Ok(e) => e,
        Err(e) => {
            warn!(path = %repo_skills.display(), error = %e, "Failed to read repo .claude/skills");
            return 0;
        }
    };

    let mut created = 0usize;
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name();
        let dst = dst_skills.join(&name);
        if std::fs::symlink_metadata(&dst).is_ok() {
            continue;
        }
        // workspace/<agent_dir>/skills/<name> → ../../<repo-name>/.claude/skills/<name>
        let target = Path::new("..")
            .join("..")
            .join(repo_name)
            .join(".claude")
            .join("skills")
            .join(&name);
        match std::os::unix::fs::symlink(&target, &dst) {
            Ok(()) => {
                debug!(link = %dst.display(), target = %target.display(), "Bridged repo skill to workspace root");
                created += 1;
            }
            Err(e) => warn!(link = %dst.display(), error = %e, "Failed to bridge repo skill"),
        }
    }

    created
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn bridges_skills_and_docs_from_repo_to_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        let repo = ws.join("my-repo");
        fs::create_dir_all(repo.join(".claude/skills/foo")).unwrap();
        fs::write(repo.join(".claude/skills/foo/SKILL.md"), "foo").unwrap();
        fs::create_dir_all(repo.join(".claude/skills/bar")).unwrap();
        fs::write(repo.join("CLAUDE.md"), "instructions").unwrap();

        let count = bridge_repo_config_to_workspace(ws, &repo);
        // 2 skills × 2 agent dirs (.claude + .agents) + CLAUDE.md = 5.
        assert_eq!(count, 5);

        // Skills are surfaced for both agents; each parent is a real dir and each
        // subdir is a symlink resolving into the repo's `.claude/skills`.
        for agent_dir in [".claude", ".agents"] {
            let dst_skills = ws.join(agent_dir).join("skills");
            let meta = fs::symlink_metadata(&dst_skills).unwrap();
            assert!(
                meta.is_dir() && !meta.file_type().is_symlink(),
                "{agent_dir}/skills must be a real directory",
            );
            assert_eq!(
                fs::read_link(dst_skills.join("foo")).unwrap(),
                Path::new("../../my-repo/.claude/skills/foo"),
            );
            assert_eq!(
                fs::read_to_string(dst_skills.join("foo/SKILL.md")).unwrap(),
                "foo",
            );
            assert!(dst_skills.join("bar").exists());
        }
        // CLAUDE.md resolves to the repo's copy.
        assert_eq!(
            fs::read_link(ws.join("CLAUDE.md")).unwrap(),
            Path::new("my-repo/CLAUDE.md"),
        );
        assert_eq!(
            fs::read_to_string(ws.join("CLAUDE.md")).unwrap(),
            "instructions"
        );
    }

    #[test]
    fn noop_when_repo_equals_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        fs::create_dir_all(ws.join(".claude/skills/foo")).unwrap();
        assert_eq!(bridge_repo_config_to_workspace(ws, ws), 0);
    }

    #[test]
    fn noop_when_repo_not_direct_child() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        let nested = ws.join("a/b");
        fs::create_dir_all(nested.join(".claude/skills/foo")).unwrap();
        assert_eq!(bridge_repo_config_to_workspace(ws, &nested), 0);
    }

    #[test]
    fn does_not_overwrite_existing_workspace_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        let repo = ws.join("repo");
        fs::create_dir_all(repo.join(".claude/skills/foo")).unwrap();
        fs::write(repo.join("CLAUDE.md"), "repo").unwrap();
        // Pre-existing authored CLAUDE.md at the workspace root must be preserved.
        fs::write(ws.join("CLAUDE.md"), "authored").unwrap();

        let count = bridge_repo_config_to_workspace(ws, &repo);
        assert_eq!(count, 2); // foo under .claude + .agents; CLAUDE.md skipped
        assert_eq!(
            fs::read_to_string(ws.join("CLAUDE.md")).unwrap(),
            "authored"
        );
        assert!(fs::read_link(ws.join("CLAUDE.md")).is_err());
    }

    #[test]
    fn is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        let repo = ws.join("repo");
        fs::create_dir_all(repo.join(".claude/skills/foo")).unwrap();
        fs::write(repo.join("CLAUDE.md"), "repo").unwrap();

        let first = bridge_repo_config_to_workspace(ws, &repo);
        let second = bridge_repo_config_to_workspace(ws, &repo);
        assert_eq!(first, 3); // foo × 2 agent dirs + CLAUDE.md
        assert_eq!(second, 0);
    }
}
