//! `--comments [--since <ref>]`: the comment-block ceiling, on its own.
//!
//! `comment-block-size` is not in `runner::RULES`, so neither the baseline nor
//! `every_registered_rule_is_enforced` holds this ceiling and this entry point
//! is the only thing that does. With `--since` it reports only a block the diff
//! touched, which is what makes a gate possible against a workspace that
//! already holds hundreds of them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use moatless_arch_rules::{
    comment_block_size::{self, Block},
    workspace::{Workspace, workspace_root},
};

/// Prints the oversize blocks in scope; `true` when it printed any.
pub(super) fn run(since: Option<&str>) -> Result<bool> {
    let root = workspace_root();
    let ws = Workspace::load(&root)?;
    let mut found = comment_block_size::blocks(&ws)?;

    if let Some(since) = since {
        let added = added_lines(&root, since)?;
        found.retain(|block| touched(block, &added));
    }

    // An annotation flags the block on the diff line that added it, which only
    // GitHub renders. Off elsewhere, where it would double the length of the
    // whole-workspace cleanup list for no gain.
    let annotate = std::env::var_os("GITHUB_ACTIONS").is_some();

    for block in &found {
        if annotate {
            println!(
                "::error file={},line={}::{}",
                block.path, block.first_line, block.violation.message
            );
        }
        println!(
            "{}:{} — {}",
            block.path, block.first_line, block.violation.key
        );
    }

    match (since, found.len()) {
        (_, 0) => println!("No comment block over its ceiling."),
        (None, n) => println!(
            "\n{n} block(s) over the ceiling in the workspace. \
             Pass --since <ref> to see only what a change added."
        ),
        (Some(since), n) => println!("\n{n} block(s) over the ceiling added since {since}."),
    }
    Ok(!found.is_empty())
}

/// Whether any line of the block was added by the diff.
///
/// The whole span, not just the opening line: adding one line to a block that
/// was at the ceiling pushes it over without touching where it starts.
fn touched(block: &Block, added: &BTreeMap<String, BTreeSet<usize>>) -> bool {
    added.get(&block.path).is_some_and(|lines| {
        lines
            .range(block.first_line..=block.last_line)
            .next()
            .is_some()
    })
}

/// Lines the diff since `since` added, per workspace-relative path.
///
/// `--merge-base` against the working tree, not `{since}...HEAD`: it keeps the
/// same exclusion of commits the base picked up meanwhile, but diffs the files
/// [`comment_block_size::blocks`] read. Against committed state an uncommitted
/// edit shifts every line below it, hiding a block not yet committed.
fn added_lines(root: &Path, since: &str) -> Result<BTreeMap<String, BTreeSet<usize>>> {
    let out = Command::new("git")
        .current_dir(root)
        // The prefixes and `core.quotePath` are pinned because `parse_diff`
        // reads them: `diff.noprefix`, `diff.mnemonicPrefix` and a non-ASCII
        // path each rewrite the `+++` line, and all three are ordinary personal
        // settings a developer machine would carry into this.
        .args([
            "-c",
            "core.quotePath=false",
            "diff",
            "--unified=0",
            "--no-color",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--diff-filter=d",
            "--merge-base",
            since,
            "--",
            "*.rs",
        ])
        .output()
        .context("run git diff")?;
    if !out.status.success() {
        bail!(
            "git diff --merge-base {since} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    parse_diff(&String::from_utf8_lossy(&out.stdout))
}

/// The added line numbers in a `--unified=0` diff, keyed by the path they land
/// in, read off the `+++ b/<path>` and `@@ … +<start>,<count> @@` lines.
///
/// A `+++` line this cannot read is an error, not a skip. Skipping it would
/// drop every hunk beneath it, and the caller gates on the result: a parse that
/// read nothing and a workspace with nothing to report are the same empty map.
fn parse_diff(text: &str) -> Result<BTreeMap<String, BTreeSet<usize>>> {
    let mut out: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut path: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            // A deletion, which adds no line to attribute. `--diff-filter=d`
            // already excludes these; this holds if that argument is dropped.
            if rest == "/dev/null" {
                path = None;
                continue;
            }
            let Some(name) = rest.strip_prefix("b/") else {
                bail!("cannot read the path in diff header {line:?}");
            };
            path = Some(name.to_owned());
        } else if let Some(rest) = line.strip_prefix("@@ ") {
            let Some(path) = path.as_ref() else { continue };
            let Some((start, count)) = hunk_added(rest) else {
                continue;
            };
            out.entry(path.clone())
                .or_default()
                .extend(start..start + count);
        }
    }
    Ok(out)
}

/// The `+<start>,<count>` half of a hunk header. A `count` of `0` is a pure
/// deletion and yields nothing; an absent one means the single line `@@ -1 +1 @@`
/// spells without a comma.
fn hunk_added(rest: &str) -> Option<(usize, usize)> {
    let plus = rest.split_whitespace().find(|f| f.starts_with('+'))?;
    let spec = plus.trim_start_matches('+');
    let (start, count) = match spec.split_once(',') {
        Some((start, count)) => (start, count.parse::<usize>().ok()?),
        None => (spec, 1),
    };
    let start = start.parse::<usize>().ok()?;
    if count == 0 {
        None
    } else {
        Some((start, count))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "fixtures must parse")]
mod tests {
    use super::*;

    fn block(path: &str, first: usize, last: usize) -> Block {
        Block {
            violation: moatless_arch_rules::Violation {
                rule: comment_block_size::RULE,
                key: format!("{path}::f"),
                message: "over".to_string(),
                locations: vec![format!("{path}:{first}")],
            },
            path: path.to_string(),
            first_line: first,
            last_line: last,
        }
    }

    #[test]
    fn a_hunk_header_yields_the_lines_it_added() {
        assert_eq!(hunk_added("-0,0 +7,3 @@"), Some((7, 3)));
        assert_eq!(hunk_added("-1 +1 @@ fn f()"), Some((1, 1)));
    }

    #[test]
    fn a_pure_deletion_adds_no_lines() {
        assert_eq!(hunk_added("-7,3 +6,0 @@"), None);
    }

    #[test]
    fn the_added_lines_are_keyed_by_the_path_they_land_in() {
        let diff = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -3,0 +4,2 @@ fn f()
+/// one
+/// two
diff --git a/b.rs b/b.rs
--- a/b.rs
+++ b/b.rs
@@ -10,0 +11,1 @@ fn g()
+/// three
";
        let added = parse_diff(diff).unwrap();
        assert_eq!(added["a.rs"], BTreeSet::from([4, 5]));
        assert_eq!(added["b.rs"], BTreeSet::from([11]));
    }

    /// The finding: `diff.noprefix` emits `+++ a.rs`, and reading that as "no
    /// path" would drop the hunks under it and exit the gate 0 having checked
    /// nothing. The command pins the prefix; this is the backstop.
    #[test]
    fn a_diff_header_without_the_expected_prefix_is_an_error() {
        let diff = "--- a.rs\n+++ a.rs\n@@ -3,0 +4,2 @@ fn f()\n+/// one\n+/// two\n";
        assert!(parse_diff(diff).is_err());
    }

    #[test]
    fn a_deleted_file_is_not_read_as_a_path() {
        let diff = "--- a/a.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-/// one\n";
        assert!(parse_diff(diff).unwrap().is_empty());
    }

    /// The finding: a block whose opening line predates the change still has to
    /// be reported when the change is what pushed it over the ceiling.
    #[test]
    fn a_block_extended_from_below_counts_as_touched() {
        let added = BTreeMap::from([("a.rs".to_string(), BTreeSet::from([9]))]);
        assert!(touched(&block("a.rs", 4, 9), &added));
    }

    #[test]
    fn a_block_the_diff_did_not_reach_is_left_alone() {
        let added = BTreeMap::from([("a.rs".to_string(), BTreeSet::from([40, 41]))]);
        assert!(!touched(&block("a.rs", 4, 9), &added));
    }

    #[test]
    fn a_block_in_an_untouched_file_is_left_alone() {
        let added = BTreeMap::from([("b.rs".to_string(), BTreeSet::from([4]))]);
        assert!(!touched(&block("a.rs", 4, 9), &added));
    }
}
