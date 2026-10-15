//! Database tests reach the shared migrations through one mechanism.

use crate::Violation;
use crate::workspace::Workspace;
use anyhow::Result;
use std::path::Path;

pub const RULE: &str = "sqlx-test-migrations";

const MIGRATIONS: &str = "crates/core/migrations";

/// A bare `#[sqlx::test]` resolves `migrations/` beside the crate's
/// `Cargo.toml`, so the symlink is what every attribute relies on.
fn links_to_core(crate_dir: &Path, root: &Path) -> bool {
    let link = crate_dir.join("migrations");
    link.is_symlink()
        && link
            .canonicalize()
            .ok()
            .zip(root.join(MIGRATIONS).canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
}

fn judge(member: &str, dir: &str, linked: bool, sources: &[(String, String)]) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut uses_sqlx_test = false;
    for (file, text) in sources {
        let lines: Vec<&str> = text.lines().collect();
        let starts: Vec<usize> = (0..lines.len())
            .filter(|&n| lines[n].trim_start().starts_with("#[sqlx::test"))
            .collect();
        uses_sqlx_test |= !starts.is_empty();
        // `migrations = false` opts out of migrations and stays allowed.
        let explicit: Vec<String> = starts
            .iter()
            .filter(|&&n| {
                let end = (n..lines.len())
                    .find(|&m| lines[m].contains(']'))
                    .unwrap_or(n);
                let attribute: String = lines[n..=end].concat();
                attribute.replace(' ', "").contains("migrations=\"")
            })
            .map(|n| format!("{file}:{}", n + 1))
            .collect();
        if !explicit.is_empty() {
            out.push(Violation {
                rule: RULE,
                key: file.clone(),
                message: "drop `migrations =` from `#[sqlx::test]`; the crate's `migrations` symlink resolves it".into(),
                locations: explicit,
            });
        }
    }
    if uses_sqlx_test && !linked {
        out.push(Violation::at(
            RULE,
            member,
            "a crate with `#[sqlx::test]` needs `migrations -> ../core/migrations` beside its Cargo.toml",
            format!("{dir}/migrations"),
        ));
    }
    out
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    for member in ws.members() {
        if ws.relative(&member.dir) == "crates/core" {
            continue;
        }
        let sources = ws.sources(member)?;
        out.extend(judge(
            &member.name,
            &ws.relative(&member.dir),
            links_to_core(&member.dir, &ws.root),
            &sources,
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(text: &str) -> Vec<(String, String)> {
        vec![("crates/x/src/db_tests.rs".into(), text.into())]
    }

    #[test]
    fn a_bare_attribute_in_a_linked_crate_passes() {
        assert!(
            judge(
                "x",
                "crates/x",
                true,
                &src("#[sqlx::test]\nasync fn t() {}")
            )
            .is_empty()
        );
    }

    #[test]
    fn an_explicit_path_is_flagged_even_where_it_resolves() {
        let found = judge(
            "x",
            "crates/x",
            true,
            &src("#[sqlx::test(migrations = \"../core/migrations\")]\nasync fn t() {}"),
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].locations, ["crates/x/src/db_tests.rs:1"]);
    }

    #[test]
    fn a_path_split_across_lines_is_flagged_and_opting_out_is_not() {
        let found = judge(
            "x",
            "crates/x",
            true,
            &src(
                "#[sqlx::test(\n    migrations = \"../m\"\n)]\nasync fn a() {}\n#[sqlx::test(migrations = false)]\nasync fn b() {}",
            ),
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].locations, ["crates/x/src/db_tests.rs:1"]);
    }

    #[test]
    fn a_crate_with_database_tests_and_no_symlink_is_flagged() {
        let found = judge(
            "x",
            "crates/x",
            false,
            &src("#[sqlx::test]\nasync fn t() {}"),
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].key, "x");
    }

    #[test]
    fn a_crate_without_database_tests_needs_no_symlink() {
        assert!(judge("x", "crates/x", false, &src("#[test]\nfn t() {}")).is_empty());
    }
}
