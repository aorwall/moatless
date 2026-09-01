//! The violations the workspace is known to carry.
//!
//! A rule lands green on a codebase that does not satisfy it yet: the
//! violations present at that moment go in the baseline, and the test fails
//! only on ones that are not. The file is a sorted list of [`Violation::id`]
//! strings, so a diff to it reads as exactly which breaches were accepted or
//! fixed.
//!
//! It only shrinks. Regenerating it to absorb a new violation is the one
//! motion this design exists to make visible.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::Violation;

pub const FILE: &str = "arch-baseline.json";

pub fn path(root: &Path) -> PathBuf {
    root.join("crates/arch-rules").join(FILE)
}

pub fn load(root: &Path) -> Result<BTreeSet<String>> {
    let file = path(root);
    if !file.exists() {
        return Ok(BTreeSet::new());
    }
    let text = std::fs::read_to_string(&file)
        .with_context(|| format!("reading {}", file.display()))?;
    let ids: Vec<String> =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", file.display()))?;
    Ok(ids.into_iter().collect())
}

pub fn write(root: &Path, violations: &[Violation]) -> Result<()> {
    let ids: BTreeSet<String> = violations.iter().map(Violation::id).collect();
    let ids: Vec<&String> = ids.iter().collect();
    let file = path(root);
    let mut text = serde_json::to_string_pretty(&ids)?;
    text.push('\n');
    std::fs::write(&file, text).with_context(|| format!("writing {}", file.display()))?;
    Ok(())
}

/// Violations that are not in the baseline, and baseline entries that no
/// longer occur. The second half is what keeps the file shrinking: a fixed
/// violation must be removed, or the next one reuses its slot.
pub struct Diff {
    pub new: Vec<Violation>,
    pub stale: Vec<String>,
}

pub fn diff(baseline: &BTreeSet<String>, found: &[Violation]) -> Diff {
    let current: BTreeSet<String> = found.iter().map(Violation::id).collect();
    Diff {
        new: found
            .iter()
            .filter(|v| !baseline.contains(&v.id()))
            .cloned()
            .collect(),
        stale: baseline.difference(&current).cloned().collect(),
    }
}
