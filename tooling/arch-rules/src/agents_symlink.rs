//! Repository instructions have one committed compatibility symlink.

use crate::Violation;
use crate::workspace::Workspace;
use anyhow::{Context, Result};
use std::path::Path;
use walkdir::WalkDir;

pub const RULE: &str = "agents-symlink";

fn valid(path: &Path) -> bool {
    path.is_symlink()
        && std::fs::read_link(path).is_ok_and(|target| target == Path::new("AGENTS.md"))
        && path.is_file()
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    for entry in WalkDir::new(&ws.root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            !matches!(
                e.file_name().to_str(),
                Some(".git" | "target" | "node_modules" | ".turbo" | ".cache" | ".worktrees")
            )
        })
    {
        let entry = entry.context("walking repository instructions")?;
        if entry.file_name() == "AGENTS.md" && entry.file_type().is_file() {
            let companion = entry.path().with_file_name("CLAUDE.md");
            if !valid(&companion) {
                out.push(Violation::at(
                    RULE,
                    ws.relative(entry.path()),
                    "CLAUDE.md must be a symlink to its sibling AGENTS.md",
                    ws.relative(&companion),
                ));
            }
        }
    }
    Ok(out)
}

#[cfg(all(test, unix))]
#[allow(
    clippy::unwrap_used,
    reason = "fixture filesystem operations must succeed"
)]
mod tests {
    use super::*;
    #[test]
    fn a_regular_file_and_a_dangling_link_do_not_qualify() {
        let dir = std::env::temp_dir().join(format!("moat-arch-symlink-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join("CLAUDE.md");
        std::fs::write(&link, "copy").unwrap();
        assert!(!valid(&link));
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("AGENTS.md", &link).unwrap();
        assert!(!valid(&link));
        std::fs::write(dir.join("AGENTS.md"), "instructions").unwrap();
        assert!(valid(&link));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
