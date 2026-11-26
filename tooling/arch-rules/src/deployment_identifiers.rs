//! No tracked file names one private deployment's hosts or ids.
//!
//! The patterns are assembled from pieces so this file does not match itself.
//! Migrations are exempt: sqlx checksums each file, so one cannot be edited.

use crate::Violation;
use crate::workspace::Workspace;
use anyhow::{Context, Result, bail};
use regex::Regex;
use std::process::Command;

pub const RULE: &str = "deployment-identifiers";

/// Each pattern with the placeholder an example uses instead.
fn patterns() -> Result<Vec<(Regex, &'static str)>> {
    let org = ["soap", "labs"].concat();
    let ids = [
        ["6290", "15357880"].concat(),
        ["9812", "69517603"].concat(),
        ["g-0a53", "093b3e"].concat(),
    ];
    let slack = [
        ["C0A9", "GP6KDLY"],
        ["C0B2", "C9P1WQK"],
        ["C0C9", "9KMF1BR"],
        ["C0CB", "SKTT6KY"],
        ["C08F", "3KAJKED"],
        ["U0AA", "QMFA9U1"],
        ["U089", "3GU35RV"],
        ["U08E", "AEEKRGQ"],
        ["T06C", "AEHGS1L"],
        ["T089", "3GU35RV"],
    ]
    .map(|id| id.concat());
    Ok(vec![
        (Regex::new(&format!(r"\b{org}test\b"))?, "`moatless.xyz`"),
        (Regex::new(&ids.join("|"))?, "an `111111111111`-style id"),
        (Regex::new(&slack.join("|"))?, "a `C0000000001`-style id"),
    ])
}

fn exempt(path: &str) -> bool {
    path.contains("/migrations/")
        && std::path::Path::new(path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sql"))
}

/// The placeholders of every pattern `text` contains.
fn breaches(patterns: &[(Regex, &'static str)], text: &str) -> Vec<&'static str> {
    patterns
        .iter()
        .filter(|(re, _)| re.is_match(text))
        .map(|(_, placeholder)| *placeholder)
        .collect()
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let out = Command::new("git")
        // CI's container owns the checkout as another user; without this, git
        // refuses it as dubious ownership and every rule test fails with it.
        .args(["-c", "safe.directory=*", "ls-files", "-z"])
        .current_dir(&ws.root)
        .output()
        .context("listing tracked files")?;
    if !out.status.success() {
        bail!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let patterns = patterns()?;
    let mut found = Vec::new();
    for path in String::from_utf8(out.stdout)?.split('\0') {
        if path.is_empty() || exempt(path) {
            continue;
        }
        // A submodule or a file deleted in the working tree reads as an error.
        let Ok(bytes) = std::fs::read(ws.root.join(path)) else {
            continue;
        };
        if bytes.iter().take(8000).any(|b| *b == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let placeholders = breaches(&patterns, &text);
        if !placeholders.is_empty() {
            found.push(Violation::at(
                RULE,
                format!("{path} names a private deployment identifier"),
                format!("replace it with {}", placeholders.join(", ")),
                path,
            ));
        }
    }
    Ok(found)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "the fixed patterns compile")]
mod tests {
    use super::*;

    #[test]
    fn a_private_host_breaches_and_a_placeholder_does_not() {
        let patterns = patterns().unwrap();
        let host = ["moatless.soap", "labstest.com"].concat();
        assert_eq!(breaches(&patterns, &host), ["`moatless.xyz`"]);
        let channel = ["C0C9", "9KMF1BR"].concat();
        assert_eq!(breaches(&patterns, &channel), ["a `C0000000001`-style id"]);
        assert!(breaches(&patterns, "moatless.xyz and C0000000001").is_empty());
    }

    #[test]
    fn a_migration_is_exempt() {
        assert!(exempt("crates/core/migrations/0038_x.sql"));
        assert!(!exempt("crates/core/src/migrations.rs"));
    }
}
