//! Words a description write sent that Linear did not store.
//!
//! Linear converts markdown to its own document model and back, so the stored
//! text never matches the sent bytes: it escapes `~`, wraps bare URLs in
//! links, renumbers lists, re-hosts images. None of that drops a word, so the
//! check compares words, and it skips URLs, HTML entities and emoji
//! shortcodes, which Linear rewrites legitimately.

use std::collections::HashSet;
use std::fmt::Write as _;

/// What a caller reads when Linear stored a description missing words it was
/// sent, or `None` when every word landed.
pub(crate) fn lost_words_warning(sent: &str, stored: &str) -> Option<String> {
    let stored: HashSet<String> = words(stored).into_iter().collect();
    let mut seen = HashSet::new();
    let lost: Vec<String> = words(sent)
        .into_iter()
        .filter(|w| !stored.contains(w) && seen.insert(w.clone()))
        .collect();
    if lost.is_empty() {
        return None;
    }
    const SHOWN: usize = 10;
    let mut shown = lost
        .iter()
        .take(SHOWN)
        .map(|w| format!("`{w}`"))
        .collect::<Vec<_>>()
        .join(", ");
    if lost.len() > SHOWN {
        let _ = write!(shown, " and {} more", lost.len() - SHOWN);
    }
    let cause = if has_indented_table_row(sent) {
        " Linear drops the first characters of every data-row cell in a table \
         indented inside a list item: move the table to column 0 and write the \
         description again."
    } else {
        ""
    };
    Some(format!(
        "The write landed, but Linear stored a description missing {} word(s) \
         it was sent: {shown}.{cause} Reading the description back and writing \
         it again loses more.",
        lost.len()
    ))
}

/// A line that opens a table row after two or more spaces, the layout Linear
/// corrupts.
fn has_indented_table_row(text: &str) -> bool {
    text.lines().any(|line| {
        let body = line.trim_start_matches(' ');
        line.len() - body.len() >= 2 && body.starts_with('|')
    })
}

/// The text's words, lowercased, with URLs, `&name;` entities and `:name:`
/// shortcodes removed.
fn words(text: &str) -> Vec<String> {
    strip_rewritten_spans(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn strip_rewritten_spans(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let skip = if rest.starts_with("http://") || rest.starts_with("https://") {
            rest.find(|c: char| c.is_whitespace() || matches!(c, ')' | '>' | ']' | '"' | '\''))
                .unwrap_or(rest.len())
        } else if c == '&' {
            delimited(rest, ';', |c| c.is_ascii_alphanumeric() || c == '#')
        } else if c == ':' {
            delimited(rest, ':', |c| {
                c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')
            })
        } else {
            0
        };
        if skip > 0 {
            out.push(' ');
            rest = &rest[skip..];
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// The byte length of `rest` up to and including `close` when every character
/// between its first one and `close` satisfies `inner`, or `0`.
fn delimited(rest: &str, close: char, inner: impl Fn(char) -> bool) -> usize {
    let body = &rest[1..];
    match body.find(|c: char| !inner(c)) {
        Some(end) if end > 0 && body[end..].starts_with(close) => end + 2,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout OP-821 lost cells from, as sent and as Linear stored it.
    const SENT: &str = "1. **Traces:**\n\n   | Trace | Root span |\n   | -- | -- |\n   \
                        | `dfcb9c2fcbb9576aeb53f2e734c7c2b5` | `enrichment-processor-chain` |\n";
    const STORED: &str = "1. **Traces:**\n\n   | Trace | Root span |\n   | -- | -- |\n   \
                          | cb9c2fcbb9576aeb53f2e734c7c2b5\\` | richment-processor-chain\\` |\n";

    #[test]
    fn a_truncated_table_cell_names_the_lost_words_and_the_cause() {
        let warning = lost_words_warning(SENT, STORED).expect("cells lost characters");
        assert!(
            warning.contains("`dfcb9c2fcbb9576aeb53f2e734c7c2b5`"),
            "{warning}"
        );
        assert!(warning.contains("`enrichment`"), "{warning}");
        assert!(warning.contains("column 0"), "{warning}");
    }

    #[test]
    fn a_description_stored_as_sent_passes() {
        assert_eq!(lost_words_warning(SENT, SENT), None);
    }

    /// Rewrites observed from Linear on a description that lost nothing.
    #[test]
    fn linear_normalisation_is_not_a_loss() {
        let sent = "1. one\n1. again\n\n- [x] done\n\nA &amp; B :tada: ~tilde~ \
                    https://example.com/a?q=1&r=2 ![img](https://example.com/a.png)\n\
                    Title\n=====\n";
        let stored = "1. one\n2. again\n\n- [X] done\n\nA & B 🎉 \\~tilde\\~ \
                      [https://example.com/a?q=1&r=2](<https://example.com/a?q=1&r=2>) \
                      ![img](https://uploads.linear.app/1/2/3)\n# Title\n";
        assert_eq!(lost_words_warning(sent, stored), None);
    }

    #[test]
    fn a_loss_outside_a_table_names_no_cause() {
        let warning = lost_words_warning("alpha beta", "alpha").expect("beta is gone");
        assert!(warning.contains("`beta`"), "{warning}");
        assert!(!warning.contains("column 0"), "{warning}");
    }
}
