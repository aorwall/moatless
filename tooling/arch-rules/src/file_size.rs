//! A source file stays under a line limit.

use anyhow::Result;

use crate::Violation;
use crate::workspace::Workspace;

pub const RULE: &str = "file-size";

/// The longest a source file may be.
const LIMIT: usize = 1600;

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut sizes = Vec::new();
    for member in ws.source_members() {
        sizes.extend(
            ws.sources(member)?
                .into_iter()
                .map(|(path, text)| (path, text.lines().count())),
        );
    }
    Ok(check_sizes(&sizes))
}

fn check_sizes(sizes: &[(String, usize)]) -> Vec<Violation> {
    sizes
        .iter()
        .filter(|(_, lines)| *lines > LIMIT)
        .map(|(path, lines)| {
            Violation::at(
                RULE,
                // The file is the breach, so the path is the whole identity.
                // A count in the key would move the file to a fresh baseline
                // slot on every edit.
                path.clone(),
                format!("file is {lines} lines; the limit is {LIMIT}"),
                path.clone(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(sizes: &[(&str, usize)]) -> Vec<Violation> {
        check_sizes(
            &sizes
                .iter()
                .map(|(path, lines)| ((*path).to_string(), *lines))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn a_file_at_the_limit_passes_and_one_past_it_does_not() {
        let violations = found(&[
            ("crates/x/src/small.rs", 12),
            ("crates/x/src/exact.rs", LIMIT),
            ("crates/x/src/over.rs", LIMIT + 1),
        ]);
        assert_eq!(
            violations.iter().map(|v| &v.key).collect::<Vec<_>>(),
            ["crates/x/src/over.rs"]
        );
    }

    /// The finding: two edits to one oversized file must land on one baseline
    /// entry, so the count belongs in the message and never in the key.
    #[test]
    fn growing_an_oversized_file_keeps_its_baseline_identity() {
        let before = found(&[("crates/x/src/over.rs", LIMIT + 1)]);
        let after = found(&[("crates/x/src/over.rs", LIMIT + 900)]);
        assert_eq!(before[0].id(), after[0].id());
        assert_ne!(before[0].message, after[0].message);
    }
}
