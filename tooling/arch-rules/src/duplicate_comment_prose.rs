//! Two files whose comment prose overlaps heavily.
//!
//! `AGENTS.md` asks that a doc never be copied between siblings and that a
//! design fact be stated once, on the symbol that owns it. This catches both
//! shapes of the same mistake: doc text copied between sibling implementations,
//! and a fact stated in both an `AGENTS.md` and the rustdoc it describes.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::Violation;
use crate::workspace::{Workspace, agents_md_files, workspace_root};

pub const RULE: &str = "duplicate-comment-prose";

/// A shared run of this many words. Long enough that "the same way" isn't a
/// hit, short enough to catch a copied sentence rather than only a copied
/// paragraph.
const SHINGLE_LEN: usize = 7;

/// Two files sharing more distinct shingles than this are restating one design
/// fact, not converging by coincidence.
const SHARED_CEILING: usize = 60;

const EXCLUDED_DIRS: &[&str] = &["node_modules", "target", ".git"];

pub fn check(_ws: &Workspace) -> Result<Vec<Violation>> {
    let root = workspace_root();
    let mut corpus = rust_comment_corpus(&root)?;
    corpus.extend(agents_md_corpus(&root)?);
    Ok(find_pairs(&corpus))
}

fn walk_excluding(root: &Path) -> impl Iterator<Item = Result<walkdir::DirEntry>> {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            !e.file_name()
                .to_str()
                .is_some_and(|name| EXCLUDED_DIRS.contains(&name))
        })
        .map(|e| e.map_err(anyhow::Error::from))
}

/// Comment prose (`//`, `///` and `//!` lines alike) of every `.rs` file under
/// `crates/`.
fn rust_comment_corpus(root: &Path) -> Result<Vec<(String, String)>> {
    let crates_dir = root.join("crates");
    if !crates_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in walk_excluding(&crates_dir) {
        let entry = entry.context("walking crates/ for comment prose")?;
        if !entry.file_type().is_file() || entry.path().extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(entry.path())
            .with_context(|| format!("reading {}", entry.path().display()))?;
        out.push((relative(root, entry.path()), rust_comment_prose(&text)));
    }
    Ok(out)
}

/// Body prose, fenced code blocks excluded, of every `AGENTS.md` in the
/// repository.
fn agents_md_corpus(root: &Path) -> Result<Vec<(String, String)>> {
    Ok(agents_md_files(root)?
        .into_iter()
        .map(|(path, text)| (path, agents_md_prose(&text)))
        .collect())
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Strip the leading `//`, `///` or `//!` from every comment line; anything
/// else in the file contributes nothing to the prose corpus.
fn rust_comment_prose(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let stripped = trimmed
            .strip_prefix("//!")
            .or_else(|| trimmed.strip_prefix("///"))
            .or_else(|| trimmed.strip_prefix("//"));
        if let Some(prose) = stripped {
            out.push_str(prose);
            out.push('\n');
        }
    }
    out
}

/// Every line outside a fenced (` ``` `) code block.
fn agents_md_prose(text: &str) -> String {
    let mut out = String::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Lowercase word tokens, punctuation and markdown markup dropped.
fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            current.extend(c.to_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn shingles(tokens: &[String]) -> HashSet<String> {
    if tokens.len() < SHINGLE_LEN {
        return HashSet::new();
    }
    (0..=tokens.len() - SHINGLE_LEN)
        .map(|i| tokens[i..i + SHINGLE_LEN].join(" "))
        .collect()
}

/// Every pair of files sharing more than [`SHARED_CEILING`] distinct shingles,
/// tallied through an inverted shingle-to-files map rather than by comparing
/// every pair.
fn find_pairs(corpus: &[(String, String)]) -> Vec<Violation> {
    let sets: Vec<HashSet<String>> = corpus
        .iter()
        .map(|(_, prose)| shingles(&tokenize(prose)))
        .collect();

    let mut by_shingle: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, set) in sets.iter().enumerate() {
        for shingle in set {
            by_shingle.entry(shingle.as_str()).or_default().push(i);
        }
    }

    let mut shared: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for files in by_shingle.values() {
        for a in 0..files.len() {
            for b in (a + 1)..files.len() {
                *shared.entry((files[a], files[b])).or_default() += 1;
            }
        }
    }

    let mut out: Vec<Violation> = shared
        .into_iter()
        .filter(|(_, count)| *count > SHARED_CEILING)
        .map(|((a, b), count)| {
            let mut paths = [corpus[a].0.clone(), corpus[b].0.clone()];
            paths.sort();
            let [first, second] = paths;
            Violation {
                rule: RULE,
                key: format!("{first}|{second}"),
                message: format!(
                    "{count} shared comment shingles, over the {SHARED_CEILING}-shingle threshold"
                ),
                locations: vec![first, second],
            }
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_short_files_share_no_shingles_below_the_word_count() {
        let corpus = vec![
            ("a.rs".to_string(), "// one two three\n".to_string()),
            ("b.rs".to_string(), "// one two three\n".to_string()),
        ];
        assert!(find_pairs(&corpus).is_empty());
    }

    #[test]
    fn a_pair_just_under_the_threshold_is_not_reported() {
        // 66 shared words yield 66 - 7 + 1 = 60 shared shingles: at, not over.
        let shared_words: Vec<String> = (0..66).map(|i| format!("w{i}")).collect();
        let tail_a: Vec<String> = (0..10).map(|i| format!("a{i}")).collect();
        let tail_b: Vec<String> = (0..10).map(|i| format!("b{i}")).collect();
        let text_a = format!("// {} {}\n", shared_words.join(" "), tail_a.join(" "));
        let text_b = format!("// {} {}\n", shared_words.join(" "), tail_b.join(" "));
        let corpus = vec![("a.rs".to_string(), text_a), ("b.rs".to_string(), text_b)];
        assert!(find_pairs(&corpus).is_empty());
    }

    #[test]
    fn a_pair_just_over_the_threshold_is_reported() {
        let shared_words: Vec<String> = (0..67).map(|i| format!("w{i}")).collect();
        let tail_a: Vec<String> = (0..10).map(|i| format!("a{i}")).collect();
        let tail_b: Vec<String> = (0..10).map(|i| format!("b{i}")).collect();
        let text_a = format!("// {} {}\n", shared_words.join(" "), tail_a.join(" "));
        let text_b = format!("// {} {}\n", shared_words.join(" "), tail_b.join(" "));
        let corpus = vec![
            ("a.rs".to_string(), text_a),
            ("b.rs".to_string(), text_b),
        ];
        // 67 shared words yield 61 shared shingles: over the 60 threshold.
        let found = find_pairs(&corpus);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].key, "a.rs|b.rs");
        assert!(found[0].message.contains("61"));
    }

    #[test]
    fn the_key_sorts_the_two_paths_regardless_of_input_order() {
        let shared_words: Vec<String> = (0..67).map(|i| format!("w{i}")).collect();
        let text = format!("// {}\n", shared_words.join(" "));
        let corpus = vec![
            ("z.rs".to_string(), text.clone()),
            ("a.rs".to_string(), text),
        ];
        let found = find_pairs(&corpus);
        assert_eq!(found[0].key, "a.rs|z.rs");
    }

    #[test]
    fn markdown_markup_and_punctuation_do_not_join_words() {
        let tokens = tokenize("**Bold** [link](url), and `code`.");
        assert_eq!(tokens, vec!["bold", "link", "url", "and", "code"]);
    }

    #[test]
    fn a_fenced_code_block_is_excluded_from_agents_md_prose() {
        let text = "Prose before.\n```rust\nfn code() {}\n```\nProse after.\n";
        let prose = agents_md_prose(text);
        assert!(prose.contains("Prose before."));
        assert!(prose.contains("Prose after."));
        assert!(!prose.contains("fn code"));
    }

    #[test]
    fn bare_line_comments_contribute_prose_here_unlike_the_block_size_rule() {
        let prose = rust_comment_prose("// a bare comment\n/// a doc comment\n//! a module doc\n");
        assert!(prose.contains("a bare comment"));
        assert!(prose.contains("a doc comment"));
        assert!(prose.contains("a module doc"));
    }
}
