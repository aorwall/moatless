//! A port of T3's `injectHtmlRenderBootstrap` (`packages/shared/src/htmlRender.ts`),
//! which gives an inline Artifact the theme and the height reporting the thread's
//! frame expects. The `<style>` and `<script>` are vendored by
//! `scripts/vendor-html-render-bootstrap.sh`; the tests pin the placement to
//! upstream's output, so regenerate both from the same t3code checkout.

const STYLE_AND_SCRIPT: &str = include_str!("html_render_bootstrap.html");
const CHARSET_META: &str = r#"<meta charset="utf-8">"#;
const VIEWPORT_META: &str =
    r#"<meta name="viewport" content="width=device-width, initial-scale=1">"#;
const RAW_TEXT_TAGS: [&[u8]; 9] = [
    b"script",
    b"style",
    b"textarea",
    b"title",
    b"xmp",
    b"iframe",
    b"noembed",
    b"noframes",
    b"noscript",
];

/// Returns `html` with the bootstrap in its head, adding a head where it has none.
pub(crate) fn inject(html: &[u8]) -> Vec<u8> {
    let scan = blank_non_markup(html);
    let markup = bootstrap_markup(&scan);
    into_head(html, &scan, &markup)
}

/// The `moatless.artifact` page script, minified by hand; `__CONFIG__` is
/// replaced by [`inject_data_script`].
const DATA_SCRIPT: &str = include_str!("data_script.js");

/// What the `moatless.artifact` script of one reader's page is configured
/// with, as the script reads it.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DataScriptConfig<'a> {
    pub(crate) token: &'a str,
    pub(crate) artifact_id: &'a str,
    /// The reader's login.
    pub(crate) viewer: &'a str,
    /// The reader's user id, which their records' `submittedBy` holds.
    pub(crate) viewer_id: &'a str,
    pub(crate) rules: Option<&'a super::rules::RecordRules>,
    /// The scoped collections `moatless.artifact.collection(name)` reads.
    pub(crate) collections: Vec<PageCollection<'a>>,
}

/// A scoped collection as the page script names it.
#[derive(serde::Serialize)]
pub(crate) struct PageCollection<'a> {
    pub(crate) scope: moatless_types::memory::MemoryScope,
    pub(crate) name: &'a str,
}

impl<'a> From<&'a super::models::AttachedCollection> for PageCollection<'a> {
    fn from(attached: &'a super::models::AttachedCollection) -> Self {
        Self {
            scope: attached.scope,
            name: &attached.name,
        }
    }
}

/// Returns `html` with the `moatless.artifact` script, configured for one
/// reader's page token, at the start of its head.
pub(crate) fn inject_data_script(html: &[u8], config: &DataScriptConfig<'_>) -> Vec<u8> {
    // `null` leaves the page without `moatless.artifact`, which a page already
    // handles, rather than a script that fails to parse.
    let config = serde_json::to_string(config)
        .unwrap_or_else(|error| {
            tracing::error!(%error, "Failed to encode an artifact page's data script config");
            "null".to_string()
        })
        // A `</script>` inside a JSON string still ends the script element.
        .replace('<', "\\u003c");
    let markup = format!(
        "<script>{}</script>",
        DATA_SCRIPT.trim_end().replace("__CONFIG__", &config)
    );
    into_head(html, &blank_non_markup(html), &markup)
}

fn into_head(html: &[u8], scan: &[u8], markup: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(html.len() + markup.len() + 32);
    if let Some(at) = find_open_tag(scan, b"head") {
        out.extend_from_slice(&html[..at]);
        out.extend_from_slice(markup.as_bytes());
        out.extend_from_slice(&html[at..]);
        return out;
    }
    let at = find_open_tag(scan, b"html").or_else(|| doctype_end(html));
    match at {
        Some(at) => out.extend_from_slice(&html[..at]),
        None => out.extend_from_slice(b"<!doctype html>"),
    }
    out.extend_from_slice(b"<head>");
    out.extend_from_slice(markup.as_bytes());
    out.extend_from_slice(b"</head>");
    out.extend_from_slice(&html[at.unwrap_or(0)..]);
    out
}

fn bootstrap_markup(scan: &[u8]) -> String {
    let mut markup = String::new();
    if !meta_tags(&scan[..scan.len().min(4096)]).any(has_charset) {
        markup.push_str(CHARSET_META);
    }
    if !meta_tags(scan).any(names_viewport) {
        markup.push_str(VIEWPORT_META);
    }
    markup.push_str(STYLE_AND_SCRIPT);
    markup
}

/// Blanks comments, raw text and template contents to spaces of the same length,
/// so offsets still line up and tags inside them cannot receive the bootstrap.
fn blank_non_markup(html: &[u8]) -> Vec<u8> {
    let mut scan = html.to_vec();
    let mut i = 0;
    while i < scan.len() {
        let end = if starts_with_ci(&scan[i..], b"<!--") {
            find_ci(&scan, i + 4, b"-->").map_or(scan.len(), |at| at + 3)
        } else if let Some(tag) = RAW_TEXT_TAGS.iter().find(|tag| opens(&scan, i, tag)) {
            raw_text_end(&scan, i + 1 + tag.len(), tag)
        } else if opens(&scan, i, b"plaintext") {
            scan.len()
        } else {
            i += 1;
            continue;
        };
        scan[i..end].fill(b' ');
        i = end;
    }
    blank_templates(&mut scan);
    scan
}

fn blank_templates(scan: &mut [u8]) {
    let mut depth = 0usize;
    let mut start = 0;
    let mut i = 0;
    while i < scan.len() {
        let Some((closing, end)) = template_tag(scan, i) else {
            i += 1;
            continue;
        };
        if !closing {
            if depth == 0 {
                start = i;
            }
            depth += 1;
        } else if depth > 0 {
            depth -= 1;
            if depth == 0 {
                scan[start..end].fill(b' ');
            }
        }
        i = end;
    }
    if depth > 0 {
        scan[start..].fill(b' ');
    }
}

/// Matches `<(\/?)template(?:\s[^>]*)?\/?>` at `i`, returning whether it closes
/// and where it ends.
fn template_tag(scan: &[u8], i: usize) -> Option<(bool, usize)> {
    let rest = scan.get(i..)?.strip_prefix(b"<")?;
    let (closing, rest) = match rest.strip_prefix(b"/") {
        Some(rest) => (true, rest),
        None => (false, rest),
    };
    if !starts_with_ci(rest, b"template") {
        return None;
    }
    let after = scan.len() - rest.len() + b"template".len();
    tag_end(scan, after).map(|end| (closing, end))
}

/// Ends the tag whose name stops at `at`: `(?:\s[^>]*)?\/?>`.
fn tag_end(scan: &[u8], at: usize) -> Option<usize> {
    match scan.get(at)? {
        b'>' => Some(at + 1),
        b'/' if scan.get(at + 1) == Some(&b'>') => Some(at + 2),
        c if c.is_ascii_whitespace() || *c == 0x0b => scan[at..]
            .iter()
            .position(|&c| c == b'>')
            .map(|p| at + p + 1),
        _ => None,
    }
}

/// The offset just past the first `<name>` or `<name …>` in `scan`.
fn find_open_tag(scan: &[u8], name: &[u8]) -> Option<usize> {
    (0..scan.len()).find_map(|i| {
        let rest = scan[i..].strip_prefix(b"<")?;
        if !starts_with_ci(rest, name) {
            return None;
        }
        let at = i + 1 + name.len();
        match scan.get(at)? {
            b'>' => Some(at + 1),
            c if c.is_ascii_whitespace() || *c == 0x0b => scan[at..]
                .iter()
                .position(|&c| c == b'>')
                .map(|p| at + p + 1),
            _ => None,
        }
    })
}

/// The offset just past a leading `<!doctype …>`, matched in `html` itself.
fn doctype_end(html: &[u8]) -> Option<usize> {
    let start = html.iter().position(|c| !is_space(*c))?;
    if !starts_with_ci(&html[start..], b"<!doctype") {
        return None;
    }
    html[start..]
        .iter()
        .position(|&c| c == b'>')
        .map(|p| start + p + 1)
}

/// The attribute text of each `<meta …>` tag, up to its `>` or the end of `scan`.
fn meta_tags(scan: &[u8]) -> impl Iterator<Item = &[u8]> {
    (0..scan.len()).filter_map(move |i| {
        let rest = scan[i..].strip_prefix(b"<")?;
        if !starts_with_ci(rest, b"meta") || !rest.get(4).is_some_and(|c| is_space(*c)) {
            return None;
        }
        let attrs = &rest[5..];
        Some(&attrs[..attrs.iter().position(|&c| c == b'>').unwrap_or(attrs.len())])
    })
}

fn has_charset(attrs: &[u8]) -> bool {
    find_ci(attrs, 0, b"charset").is_some()
}

/// `name\s*=\s*["']?viewport`.
fn names_viewport(attrs: &[u8]) -> bool {
    let mut from = 0;
    while let Some(at) = find_ci(attrs, from, b"name") {
        from = at + 1;
        let mut rest = &attrs[at + 4..];
        rest = trim_start(rest);
        let Some(value) = rest.strip_prefix(b"=") else {
            continue;
        };
        let value = trim_start(value);
        let value = value
            .strip_prefix(b"\"")
            .or_else(|| value.strip_prefix(b"'"))
            .unwrap_or(value);
        if starts_with_ci(value, b"viewport") {
            return true;
        }
    }
    false
}

/// Whether `<tag` opens at `i`, followed by a non-word byte or the end: `<tag\b`.
fn opens(scan: &[u8], i: usize, tag: &[u8]) -> bool {
    scan[i..].strip_prefix(b"<").is_some_and(|rest| {
        starts_with_ci(rest, tag)
            && rest
                .get(tag.len())
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_'))
    })
}

/// The end of the raw text opened before `from`: past its `</tag\s*>`, or the end.
fn raw_text_end(scan: &[u8], from: usize, tag: &[u8]) -> usize {
    let mut at = from;
    while let Some(close) = find_ci(scan, at, b"</") {
        let name_end = close + 2 + tag.len();
        if starts_with_ci(&scan[close + 2..], tag) {
            let rest = trim_start(&scan[name_end..]);
            if rest.first() == Some(&b'>') {
                return scan.len() - rest.len() + 1;
            }
        }
        at = close + 2;
    }
    scan.len()
}

fn find_ci(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    (from..haystack.len()).find(|&i| starts_with_ci(&haystack[i..], needle))
}

fn starts_with_ci(haystack: &[u8], prefix: &[u8]) -> bool {
    haystack.len() >= prefix.len() && haystack[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn trim_start(bytes: &[u8]) -> &[u8] {
    let n = bytes.iter().take_while(|c| is_space(**c)).count();
    &bytes[n..]
}

/// JavaScript's `\s` over ASCII.
fn is_space(c: u8) -> bool {
    c.is_ascii_whitespace() || c == 0x0b
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inputs and upstream's output for them, with `{B}` for [`STYLE_AND_SCRIPT`].
    const CASES: &[(&str, &str)] = &[
        (
            "<!doctype html><html><head><style>:root{--background:red}</style></head><body>x</body></html>",
            "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}<style>:root{--background:red}</style></head><body>x</body></html>",
        ),
        (
            "<meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><p>hi</p>",
            "<!doctype html><head>{B}</head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><p>hi</p>",
        ),
        (
            "<textarea><head><meta name=\"viewport\"></head></textarea>",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><textarea><head><meta name=\"viewport\"></head></textarea>",
        ),
        (
            "<title><head><meta name=\"viewport\"></head></title>",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><title><head><meta name=\"viewport\"></head></title>",
        ),
        (
            "<xmp><head><meta name=\"viewport\"></head></xmp>",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><xmp><head><meta name=\"viewport\"></head></xmp>",
        ),
        (
            "<iframe><head><meta name=\"viewport\"></head></iframe>",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><iframe><head><meta name=\"viewport\"></head></iframe>",
        ),
        (
            "<noembed><head><meta name=\"viewport\"></head></noembed>",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><noembed><head><meta name=\"viewport\"></head></noembed>",
        ),
        (
            "<noframes><head><meta name=\"viewport\"></head></noframes>",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><noframes><head><meta name=\"viewport\"></head></noframes>",
        ),
        (
            "<noscript><head><meta name=\"viewport\"></head></noscript>",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><noscript><head><meta name=\"viewport\"></head></noscript>",
        ),
        (
            "<plaintext><head><meta name=\"viewport\"></head></plaintext>",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><plaintext><head><meta name=\"viewport\"></head></plaintext>",
        ),
        (
            "<template><head><meta name=\"viewport\"></head></template>",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><template><head><meta name=\"viewport\"></head></template>",
        ),
        (
            "<template><template>inner</template><head><meta name=\"viewport\"></head></template>",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><template><template>inner</template><head><meta name=\"viewport\"></head></template>",
        ),
        (
            "<!-- copy <head> and <meta name=\"viewport\"> here --><html><head><script>const tag = '<meta name=\"viewport\">';</script></head><body>x</body></html>",
            "<!-- copy <head> and <meta name=\"viewport\"> here --><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}<script>const tag = '<meta name=\"viewport\">';</script></head><body>x</body></html>",
        ),
        (
            "<html lang=\"en\"><HEAD data-x=\"1\"><title>t</title></HEAD><body></body></html>",
            "<html lang=\"en\"><HEAD data-x=\"1\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}<title>t</title></HEAD><body></body></html>",
        ),
        (
            "<html lang=\"en\"><body>no head</body></html>",
            "<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><body>no head</body></html>",
        ),
        (
            "  <!DOCTYPE html>\n<p>only a doctype</p>",
            "  <!DOCTYPE html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head>\n<p>only a doctype</p>",
        ),
        (
            "<header>not a head</header><p>x</p>",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><header>not a head</header><p>x</p>",
        ),
        (
            "<head><meta name='viewport' content='width=1'><META CHARSET=utf-8></head>",
            "<head>{B}<meta name='viewport' content='width=1'><META CHARSET=utf-8></head>",
        ),
        (
            "<!-- unterminated <head>",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><!-- unterminated <head>",
        ),
        (
            "<script>let a = \"<head>\"",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><script>let a = \"<head>\"",
        ),
        (
            "<scriptx><head></head>",
            "<scriptx><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head>",
        ),
        (
            "<SCRIPT>\"<head>\"</SCRIPT ><head></head>",
            "<SCRIPT>\"<head>\"</SCRIPT ><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head>",
        ),
        (
            "<template id=\"a\"><head></head></template><head></head>",
            "<template id=\"a\"><head></head></template><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head>",
        ),
        (
            "<template/><head></head>",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head><template/><head></head>",
        ),
        (
            "<p>héllo ✓</p><head></head>",
            "<p>héllo ✓</p><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head>",
        ),
        (
            "<title><head></title><head></head>",
            "<title><head></title><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head>",
        ),
        (
            "",
            "<!doctype html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{B}</head>",
        ),
    ];

    fn injected(html: &str) -> String {
        String::from_utf8(inject(html.as_bytes()))
            .unwrap()
            .replace(STYLE_AND_SCRIPT, "{B}")
    }

    #[test]
    fn matches_upstream_placement() {
        for (input, expected) in CASES {
            assert_eq!(injected(input), *expected, "input: {input}");
        }
    }

    #[test]
    fn looks_for_a_charset_only_in_the_first_4096_bytes() {
        let filler = "x".repeat(5000);
        let html = format!(r#"<head>{filler}<meta charset="utf-8"></head>"#);
        assert_eq!(
            injected(&html),
            format!(
                r#"<head>{CHARSET_META}{VIEWPORT_META}{{B}}{filler}<meta charset="utf-8"></head>"#
            )
        );
    }

    #[test]
    fn vendored_fragment_is_a_style_and_a_script() {
        assert!(STYLE_AND_SCRIPT.starts_with(r#"<style id="t3-theme">"#));
        assert!(STYLE_AND_SCRIPT.ends_with("</script>"));
    }

    #[test]
    fn keeps_non_utf8_bytes() {
        let out = inject(b"<head></head>\xff");
        assert!(out.ends_with(b"</head>\xff"));
    }

    #[test]
    fn the_data_script_opens_the_head_with_its_config_escaped() {
        let out = inject_data_script(
            b"<html><head><title>x</title></head></html>",
            &DataScriptConfig {
                token: "tok.en",
                artifact_id: "art-1",
                viewer: "</script>",
                viewer_id: "user-1",
                rules: None,
                collections: Vec::new(),
            },
        );
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.starts_with("<html><head><script>(function(){var c={"),
            "{out}"
        );
        assert!(out.contains(r#""token":"tok.en""#), "{out}");
        assert!(out.contains(r#""viewerId":"user-1","rules":null"#), "{out}");
        assert!(!out.contains("__CONFIG__"), "{out}");
        assert_eq!(out.matches("</script>").count(), 1, "{out}");
        assert!(
            out.ends_with("</script><title>x</title></head></html>"),
            "{out}"
        );
    }
}
