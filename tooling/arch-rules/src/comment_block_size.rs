//! A `///` or `//!` block longer than the workspace's stated ceiling.
//!
//! Root `AGENTS.md` asks for 5 lines in a doc comment and 8 in a module doc.
//! A block costs what its text takes wrapped at [`WRAP_WIDTH`], and a blank
//! line costs nothing, so neither joining lines nor merging paragraphs buys
//! anything. A bare `//` block has no declared item to key on, so only `///`
//! and `//!` are scoped. Reads the cargo members, so `crates/codex-sdk` — its
//! own workspace — is unchecked, unlike `duplicate-comment-prose`.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;

use crate::Violation;
use crate::workspace::Workspace;

pub const RULE: &str = "comment-block-size";

/// Ceiling for a `///` block: the 5 lines root `AGENTS.md` asks for.
const DOC_CEILING: usize = 5;

/// Ceiling for a `//!` block: the 8 lines root `AGENTS.md` asks for.
const MODULE_DOC_CEILING: usize = 8;

/// The column this workspace wraps comments at, and so what one line costs.
const WRAP_WIDTH: usize = 80;

const ITEM_KEYWORDS: &[&str] = &[
    "fn",
    "struct",
    "enum",
    "trait",
    "type",
    "const",
    "static",
    "mod",
    "macro_rules!",
];

/// FNV-1a over the block's own text, short-hex. Content-derived so a block
/// keyed on it survives an insertion above it, which an ordinal would not.
fn block_hash(lines: &[&str]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for line in lines {
        for byte in line.trim().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
        hash ^= u64::from(b'\n');
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{:06x}", hash & 0xff_ffff)
}

/// An oversize block: what it breaches, and the 1-based line span it occupies.
///
/// `arch-report --comments --since` intersects that span with a diff. The
/// violation's `locations` carries only the opening line, which cannot answer
/// whether a change extended a block that already started above it.
pub struct Block {
    pub violation: Violation,
    /// Workspace-relative path, which a diff is matched against by path.
    pub path: String,
    pub first_line: usize,
    pub last_line: usize,
    /// [`block_hash`] of this block's own text. Identifies the block across a
    /// run to whatever has already reported it, and changes when the text does.
    pub hash: String,
}

/// Every oversize block in the workspace, with the span each one occupies.
pub fn blocks(ws: &Workspace) -> Result<Vec<Block>> {
    let mut out = Vec::new();
    for member in ws.members() {
        for (path, text) in ws.sources(member)? {
            out.extend(blocks_in_file(&path, &text));
        }
    }
    Ok(out)
}

fn is_doc_line(trimmed: &str) -> bool {
    // Four or more slashes is an ordinary comment to rustc, not a doc line.
    trimmed.starts_with("///") && !trimmed.starts_with("////")
}

fn is_module_doc_line(trimmed: &str) -> bool {
    trimmed.starts_with("//!")
}

/// What a block costs: each line rounded up to whole [`WRAP_WIDTH`] columns, so
/// an over-wide line pays for every wrapped line it stands in for. A blank line
/// is free — it is the paragraph break rustdoc reads to find the summary, and
/// charging for it pays for merging the summary into the body, which deletes no
/// prose and breaks the rendered docs.
fn block_cost(lines: &[&str]) -> usize {
    lines
        .iter()
        .filter(|line| !is_blank_doc_line(line))
        // Columns, not bytes: an em dash is 3 bytes, and charging by length
        // bills a line already wrapped at 80 for a second line it never takes.
        .map(|line| line.trim_end().chars().count().div_ceil(WRAP_WIDTH).max(1))
        .sum()
}

/// Whether a doc line carries no text past its `///` or `//!` marker.
fn is_blank_doc_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed
        .strip_prefix("///")
        .or_else(|| trimmed.strip_prefix("//!"))
        .is_some_and(|body| body.trim().is_empty())
}

/// Every oversize block in one file, keyed by the item the block documents.
///
/// A key that two blocks in one file would share is disambiguated by
/// [`block_hash`], so one baseline entry never accepts two breaches. Two blocks
/// whose text is byte-identical hash alike, and only there does an ordinal
/// separate them — it renumbers nothing a hash could have told apart.
fn blocks_in_file(path: &str, text: &str) -> Vec<Block> {
    let found = collect_blocks(path, text);
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for block in &found {
        *counts.entry(block.violation.key.as_str()).or_default() += 1;
    }
    let shared: BTreeSet<String> = counts
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(key, _)| key.to_string())
        .collect();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = Vec::with_capacity(found.len());
    for mut block in found {
        if shared.contains(&block.violation.key) {
            block.violation.key = format!("{}@{}", block.violation.key, block.hash);
        }
        let nth = seen.entry(block.violation.key.clone()).or_default();
        *nth += 1;
        if *nth > 1 {
            block.violation.key = format!("{}#{nth}", block.violation.key);
        }
        out.push(block);
    }
    out
}

/// Every oversize block, each carrying the hash of its own text.
fn collect_blocks(path: &str, text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if is_module_doc_line(trimmed) {
            let start = i;
            let mut end = i;
            while end + 1 < lines.len() && is_module_doc_line(lines[end + 1].trim_start()) {
                end += 1;
            }
            let len = block_cost(&lines[start..=end]);
            if len > MODULE_DOC_CEILING {
                out.push(Block {
                    violation: Violation {
                        rule: RULE,
                        key: format!("{path}::module"),
                        message: format!(
                            "module doc block of {len} lines wrapped at {WRAP_WIDTH} exceeds the {MODULE_DOC_CEILING}-line ceiling"
                        ),
                        locations: vec![format!("{path}:{}", start + 1)],
                    },
                    path: path.to_string(),
                    first_line: start + 1,
                    last_line: end + 1,
                    hash: block_hash(&lines[start..=end]),
                });
            }
            i = end + 1;
        } else if is_doc_line(trimmed) {
            let start = i;
            let mut end = i;
            while end + 1 < lines.len() && is_doc_line(lines[end + 1].trim_start()) {
                end += 1;
            }
            let len = block_cost(&lines[start..=end]);
            if len > DOC_CEILING {
                let hash = block_hash(&lines[start..=end]);
                // No declaration to name: key on the block's own text, because
                // an ordinal renumbers every later block when one is inserted
                // above it.
                let key = match find_item_name(&lines, end + 1) {
                    Some(name) => format!("{path}::{name}"),
                    None => format!("{path}::doc@{hash}"),
                };
                out.push(Block {
                    violation: Violation {
                        rule: RULE,
                        key,
                        message: format!(
                            "doc comment block of {len} lines wrapped at {WRAP_WIDTH} exceeds the {DOC_CEILING}-line ceiling"
                        ),
                        locations: vec![format!("{path}:{}", start + 1)],
                    },
                    path: path.to_string(),
                    first_line: start + 1,
                    last_line: end + 1,
                    hash,
                });
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// The declaration a doc block documents, skipping blank lines and any
/// attributes between the block and the item.
fn find_item_name(lines: &[&str], mut idx: usize) -> Option<String> {
    while idx < lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed.is_empty() {
            idx += 1;
            continue;
        }
        if trimmed.starts_with("#[") || trimmed.starts_with("#![") {
            let mut depth = i32::try_from(trimmed.matches('[').count()).ok()?
                - i32::try_from(trimmed.matches(']').count()).ok()?;
            idx += 1;
            while depth > 0 && idx < lines.len() {
                depth += i32::try_from(lines[idx].matches('[').count()).ok()?
                    - i32::try_from(lines[idx].matches(']').count()).ok()?;
                idx += 1;
            }
            continue;
        }
        break;
    }
    let line = *lines.get(idx)?;
    parse_item_name(line).or_else(|| parse_field_or_variant(line))
}

fn generic_skip(tokens: &[&str]) -> usize {
    let mut idx = 0;
    while idx < tokens.len() {
        let tok = tokens[idx];
        let is_modifier = tok == "pub"
            || tok.starts_with("pub(")
            || tok == "async"
            || tok == "unsafe"
            || tok == "extern"
            || tok == "default"
            || tok.starts_with('"');
        if is_modifier {
            idx += 1;
        } else {
            break;
        }
    }
    idx
}

fn extract_ident(tok: &str) -> Option<String> {
    let name: String = tok
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

/// The name of the `fn`/`struct`/`enum`/`trait`/`type`/`const`/`static`/`mod`
/// this line declares, or the participants of an `impl` block.
fn parse_item_name(line: &str) -> Option<String> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let mut idx = generic_skip(&tokens);
    let mut keyword = *tokens.get(idx)?;
    // `const fn` names the function, not a const item.
    if keyword == "const" && tokens.get(idx + 1) == Some(&"fn") {
        idx += 1;
        keyword = "fn";
    }
    idx += 1;
    // `impl<T>` binds the generics to the keyword with no space, so the token is
    // the whole `impl<...>` and an equality test would miss every generic impl.
    if keyword == "impl" || keyword.starts_with("impl<") {
        let rest = tokens.get(idx..)?.join(" ");
        let rest = rest.split('{').next().unwrap_or_default().trim();
        let rest = rest.split(" where").next().unwrap_or(rest).trim();
        return if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        };
    }
    if !ITEM_KEYWORDS.contains(&keyword) {
        return None;
    }
    extract_ident(tokens.get(idx)?)
}

/// A struct field or enum variant name: an identifier followed by the shape of
/// a field (`: Type`) or a variant (`(..)`, `{..}`, `,`, `=`, or end of line),
/// tried only once `parse_item_name` finds no keyword.
fn parse_field_or_variant(line: &str) -> Option<String> {
    let mut rest = line.trim();
    for prefix in ["pub(crate)", "pub(super)", "pub(self)"] {
        if let Some(r) = rest.strip_prefix(prefix) {
            rest = r.trim_start();
        }
    }
    if let Some(r) = rest.strip_prefix("pub") {
        if r.is_empty() || r.starts_with(char::is_whitespace) {
            rest = r.trim_start();
        }
    }
    let ident: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if ident.is_empty() {
        return None;
    }
    let after = rest[ident.len()..].trim_start();
    let shaped_like_a_member = (after.starts_with(':') && !after.starts_with("::"))
        || after.starts_with('(')
        || after.starts_with('{')
        || after.starts_with(',')
        || after.starts_with('=')
        || after.is_empty();
    if shaped_like_a_member { Some(ident) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(path: &str, text: &str) -> Vec<String> {
        blocks_in_file(path, text)
            .into_iter()
            .map(|b| b.violation.key)
            .collect()
    }

    #[test]
    fn a_doc_block_at_the_ceiling_passes() {
        let text = format!(
            "{}\npub fn f() {{}}\n",
            "/// line\n".repeat(DOC_CEILING).trim_end()
        );
        assert!(keys("x.rs", &text).is_empty(), "{text}");
    }

    #[test]
    fn one_line_over_the_ceiling_fails() {
        let text = format!(
            "{}\npub fn f() {{}}\n",
            "/// line\n".repeat(DOC_CEILING + 1).trim_end()
        );
        assert_eq!(keys("x.rs", &text), vec!["x.rs::f"]);
    }

    /// The finding: charging for the paragraph break paid for merging the
    /// summary into the body, which deletes no prose and costs the rendered
    /// docs their summary line.
    #[test]
    fn deleting_the_blank_line_between_paragraphs_does_not_clear_a_block() {
        let over = format!(
            "/// summary\n///\n{}pub fn f() {{}}\n",
            "/// line\n".repeat(DOC_CEILING)
        );
        let merged = over.replace("///\n", "");
        assert_eq!(keys("x.rs", &over), vec!["x.rs::f"]);
        assert_eq!(keys("x.rs", &merged), vec!["x.rs::f"]);
    }

    #[test]
    fn joining_an_over_ceiling_block_onto_one_long_line_does_not_clear_it() {
        let long = "word ".repeat(20 * (DOC_CEILING + 1));
        let text = format!("/// {}\npub fn f() {{}}\n", long.trim_end());
        assert_eq!(keys("x.rs", &text), vec!["x.rs::f"]);
    }

    #[test]
    fn a_line_at_the_wrap_width_costs_one() {
        let at_width = "/".repeat(WRAP_WIDTH);
        assert_eq!(block_cost(&[at_width.as_str()]), 1);
        let over = "/".repeat(WRAP_WIDTH + 1);
        assert_eq!(block_cost(&[over.as_str()]), 2);
    }

    /// The finding: a byte count billed a line at the stated width for a
    /// second line it never takes, and this repo's doc style uses em dashes
    /// heavily.
    #[test]
    fn a_line_at_the_wrap_width_costs_one_however_wide_its_characters_are() {
        let with_dashes = format!("{} — —", "/".repeat(WRAP_WIDTH - 4));
        assert_eq!(with_dashes.chars().count(), WRAP_WIDTH);
        assert!(with_dashes.len() > WRAP_WIDTH);
        assert_eq!(block_cost(&[with_dashes.as_str()]), 1);
    }

    #[test]
    fn trailing_whitespace_does_not_push_a_line_over_the_wrap_width() {
        let padded = format!("{}   ", "/".repeat(WRAP_WIDTH));
        assert_eq!(block_cost(&[padded.as_str()]), 1);
    }

    #[test]
    fn a_module_doc_block_at_the_ceiling_passes() {
        let text = "//! line\n".repeat(MODULE_DOC_CEILING);
        assert!(keys("x.rs", &text).is_empty());
    }

    #[test]
    fn a_module_doc_block_one_line_over_fails() {
        let text = "//! line\n".repeat(MODULE_DOC_CEILING + 1);
        assert_eq!(keys("x.rs", &text), vec!["x.rs::module"]);
    }

    #[test]
    fn a_four_slash_run_is_not_a_doc_block() {
        let text = format!("{}\npub fn f() {{}}\n", "//// line\n".repeat(20));
        assert!(keys("x.rs", &text).is_empty());
    }

    #[test]
    fn the_key_names_the_documented_item() {
        let over = "/// line\n".repeat(DOC_CEILING + 1);
        for (decl, name) in [
            ("pub fn widget() {}", "widget"),
            ("pub struct Widget;", "Widget"),
            ("pub enum Widget {}", "Widget"),
            ("pub trait Widget {}", "Widget"),
            ("pub type Widget = u8;", "Widget"),
            ("pub const WIDGET: u8 = 1;", "WIDGET"),
            ("pub static WIDGET: u8 = 1;", "WIDGET"),
            ("pub mod widget;", "widget"),
            ("pub(crate) const fn widget() -> u8 { 1 }", "widget"),
        ] {
            let text = format!("{over}{decl}\n");
            assert_eq!(keys("x.rs", &text), vec![format!("x.rs::{name}")], "{decl}");
        }
    }

    #[test]
    fn an_attribute_between_the_block_and_the_item_is_skipped() {
        let over = "/// line\n".repeat(DOC_CEILING + 1);
        let text = format!("{over}#[derive(Debug, Clone)]\npub struct Widget;\n");
        assert_eq!(keys("x.rs", &text), vec!["x.rs::Widget"]);
    }

    #[test]
    fn a_struct_field_is_keyed_by_its_bare_name() {
        let over = "    /// line\n".repeat(DOC_CEILING + 1);
        let text = format!("pub struct S {{\n{over}    pub name: String,\n}}\n");
        assert_eq!(keys("x.rs", &text), vec!["x.rs::name"]);
    }

    /// The finding: a bare field name is unique to its struct, not to the
    /// file, so two `path` fields would otherwise share one baseline slot and
    /// fixing either would leave the other permanently accepted.
    #[test]
    fn two_fields_of_one_name_in_one_file_get_two_keys() {
        let first = "    /// first\n".repeat(DOC_CEILING + 1);
        let second = "    /// second\n".repeat(DOC_CEILING + 1);
        let text = format!(
            "pub struct A {{\n{first}    pub path: String,\n}}\n\
             pub struct B {{\n{second}    pub path: String,\n}}\n"
        );
        let found = keys("x.rs", &text);
        assert_eq!(found.len(), 2);
        assert_ne!(found[0], found[1]);
        assert!(found.iter().all(|key| key.starts_with("x.rs::path@")));
    }

    #[test]
    fn an_enum_variant_is_keyed_by_its_bare_name() {
        let over = "    /// line\n".repeat(DOC_CEILING + 1);
        let text = format!("pub enum E {{\n{over}    Variant(u8),\n}}\n");
        assert_eq!(keys("x.rs", &text), vec!["x.rs::Variant"]);
    }

    #[test]
    fn an_impl_block_is_keyed_by_its_participants() {
        let over = "/// line\n".repeat(DOC_CEILING + 1);
        let text = format!("{over}impl Trait for Widget {{\n}}\n");
        assert_eq!(keys("x.rs", &text), vec!["x.rs::Trait for Widget"]);
    }

    #[test]
    fn an_unparseable_block_falls_back_to_its_own_text() {
        let over = "/// line\n".repeat(DOC_CEILING + 1);
        let text = format!("{over}}}\n");
        let hash = block_hash(&["/// line"; DOC_CEILING + 1]);
        assert_eq!(keys("x.rs", &text), vec![format!("x.rs::doc@{hash}")]);
    }

    /// The finding: two blocks in one file must never share a baseline slot,
    /// even when nothing but their position tells them apart.
    #[test]
    fn two_identical_unparseable_blocks_get_two_keys() {
        let over = "/// line\n".repeat(DOC_CEILING + 1);
        let text = format!("{over}}}\n{over}}}\n");
        let found = keys("x.rs", &text);
        assert_eq!(found.len(), 2);
        assert_ne!(found[0], found[1]);
    }

    #[test]
    fn a_generic_impl_is_keyed_on_its_participants() {
        let over = "/// line\n".repeat(DOC_CEILING + 1);
        let text = format!("{over}impl<T> Trait for Widget<T> {{\n}}\n");
        assert_eq!(keys("x.rs", &text), vec!["x.rs::Trait for Widget<T>"]);
    }

    /// The finding: `--comments --since` intersects the span with a diff, so a
    /// span short by a line silently stops gating the line that pushed the
    /// block over.
    #[test]
    fn the_span_covers_every_line_of_the_block() {
        let over = "/// line\n".repeat(DOC_CEILING + 1);
        let text = format!("fn before() {{}}\n{over}pub fn f() {{}}\n");
        let found = blocks_in_file("x.rs", &text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].first_line, 2);
        assert_eq!(found[0].last_line, 2 + DOC_CEILING);
        assert_eq!(found[0].path, "x.rs");
    }

    #[test]
    fn a_macro_definition_is_keyed_on_its_name() {
        let over = "/// line\n".repeat(DOC_CEILING + 1);
        let text = format!("{over}macro_rules! rules {{\n}}\n");
        assert_eq!(keys("x.rs", &text), vec!["x.rs::rules"]);
    }
}
