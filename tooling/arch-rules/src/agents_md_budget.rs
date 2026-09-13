//! An `AGENTS.md` longer than the workspace's stated budget.
//!
//! Every agent reads the root `AGENTS.md` plus the nearest one on every task,
//! so length here is a context cost paid per invocation rather than per read.

use anyhow::Result;

use crate::Violation;
use crate::workspace::{Workspace, agents_md_files, workspace_root};

pub const RULE: &str = "agents-md-budget";

const LINE_CEILING: usize = 120;

/// Every oversize `AGENTS.md` in the repository.
///
/// `ws` is unread: this walks the repository tree, not cargo members.
pub fn check(_ws: &Workspace) -> Result<Vec<Violation>> {
    let files: Vec<(String, usize)> = agents_md_files(&workspace_root())?
        .into_iter()
        .map(|(path, text)| (path, text.lines().count()))
        .collect();
    Ok(check_lengths(&files))
}

fn check_lengths(files: &[(String, usize)]) -> Vec<Violation> {
    files
        .iter()
        .filter(|(_, len)| *len > LINE_CEILING)
        .map(|(path, len)| Violation {
            rule: RULE,
            key: path.clone(),
            message: format!("AGENTS.md is {len} lines, over the {LINE_CEILING}-line budget"),
            locations: vec![path.clone()],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(files: &[(&str, usize)]) -> Vec<String> {
        check_lengths(
            &files
                .iter()
                .map(|(p, n)| ((*p).to_string(), *n))
                .collect::<Vec<_>>(),
        )
        .into_iter()
        .map(|v| v.key)
        .collect()
    }

    #[test]
    fn a_file_at_the_ceiling_passes() {
        assert!(keys(&[("AGENTS.md", LINE_CEILING)]).is_empty());
    }

    #[test]
    fn one_line_over_the_ceiling_fails() {
        assert_eq!(
            keys(&[("AGENTS.md", LINE_CEILING + 1)]),
            vec!["AGENTS.md"]
        );
    }

    #[test]
    fn only_the_oversize_files_are_reported() {
        assert_eq!(
            keys(&[
                ("crates/a/AGENTS.md", 10),
                ("crates/b/AGENTS.md", LINE_CEILING + 5),
            ]),
            vec!["crates/b/AGENTS.md"]
        );
    }
}
