//! `LOOP.md` frontmatter + body parser.
//!
//! Mirrors [`crate::skill::scanner`] but keeps the markdown body after the
//! closing `---` as the loop's prompt. Individual parse failures are logged and
//! returned as `None` so one malformed file doesn't tank the whole sync.

use tracing::warn;

use super::models::{LoopFrontmatter, ParsedLoop};

/// Parse a `LOOP.md` body into a [`ParsedLoop`].
///
/// Returns `None` (after logging a warning) when the frontmatter is missing,
/// malformed, or omits the required `on` field. `dir_name` becomes the loop's
/// stable name.
pub fn parse_loop(content: &str, dir_name: &str) -> Option<ParsedLoop> {
    let (yaml, body) = match split_frontmatter(content) {
        Ok(parts) => parts,
        Err(e) => {
            warn!(dir_name, error = %e, "Skipping loop: malformed frontmatter");
            return None;
        }
    };

    let frontmatter: LoopFrontmatter = match serde_yaml::from_str(&yaml) {
        Ok(fm) => fm,
        Err(e) => {
            warn!(dir_name, error = %e, "Skipping loop: invalid frontmatter");
            return None;
        }
    };

    if frontmatter.on.trim().is_empty() {
        warn!(dir_name, "Skipping loop: missing `on` field");
        return None;
    }

    Some(ParsedLoop {
        name: dir_name.to_string(),
        frontmatter,
        prompt_body: body.trim().to_string(),
    })
}

/// Split a `LOOP.md` into `(yaml_frontmatter, markdown_body)`.
///
/// Tolerates a leading UTF-8 BOM and CRLF line endings, like the skill scanner.
fn split_frontmatter(contents: &str) -> Result<(String, String), String> {
    // Strip the optional UTF-8 BOM so `---` lands at byte 0.
    let trimmed = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    // Frontmatter must start at the very first line.
    let rest = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))
        .ok_or_else(|| "missing opening '---' delimiter".to_string())?;

    // Find the closing delimiter: a line that is exactly `---`.
    let mut end_byte = None;
    let mut after_byte = 0usize;
    let mut cursor = 0usize;
    for line in rest.split_inclusive('\n') {
        let line_content = line.trim_end_matches(['\n', '\r']);
        if line_content == "---" {
            end_byte = Some(cursor);
            after_byte = cursor + line.len();
            break;
        }
        cursor += line.len();
    }
    let end_byte = end_byte.ok_or_else(|| "missing closing '---' delimiter".to_string())?;
    let yaml = rest[..end_byte].to_string();
    let body = rest[after_byte..].to_string();
    Ok((yaml, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_pr_loop_with_body() {
        let content = "---\non: github_pr\ntags: [review]\n---\n\nReview this PR.\n";
        let loop_ = parse_loop(content, "java-review").expect("valid loop parses");
        assert_eq!(loop_.name, "java-review");
        assert_eq!(loop_.frontmatter.on, "github_pr");
        assert_eq!(loop_.frontmatter.tags.as_deref(), Some(&["review".to_string()][..]));
        assert_eq!(loop_.prompt_body, "Review this PR.");
        // Absent inactiveTtlDays inherits the platform default (None).
        assert_eq!(loop_.frontmatter.inactive_ttl_days, None);
    }

    #[test]
    fn parses_inactive_ttl_days() {
        let content = "---\non: github_pr\ninactiveTtlDays: 1\n---\n\nbody\n";
        let loop_ = parse_loop(content, "java-review").expect("valid loop parses");
        assert_eq!(loop_.frontmatter.inactive_ttl_days, Some(1));
    }

    #[test]
    fn parses_schedule_loop() {
        let content = "---\non: schedule\nschedule:\n  cron: \"0 0 6 * * MON-FRI *\"\n  timezone: Europe/Stockholm\n  message: \"Run the health check.\"\n---\n\nRun the daily-health-check skill.\n";
        let loop_ = parse_loop(content, "daily-health-check").expect("schedule loop parses");
        assert_eq!(loop_.frontmatter.on, "schedule");
        let sched = loop_.frontmatter.schedule.as_ref().expect("schedule block");
        assert_eq!(sched.cron.as_deref(), Some("0 0 6 * * MON-FRI *"));
        assert_eq!(sched.timezone.as_deref(), Some("Europe/Stockholm"));
        assert_eq!(sched.message.as_deref(), Some("Run the health check."));
        assert_eq!(loop_.prompt_body, "Run the daily-health-check skill.");
    }

    #[test]
    fn skips_loop_missing_on() {
        // `on` is required by serde — missing key fails deserialization.
        let content = "---\nagentType: claude-code\n---\n\nbody\n";
        assert!(parse_loop(content, "no-on").is_none());
    }

    #[test]
    fn skips_loop_without_frontmatter() {
        assert!(parse_loop("# just markdown\n", "no-fm").is_none());
    }

    #[test]
    fn skips_loop_with_unclosed_frontmatter() {
        assert!(parse_loop("---\non: github_pr\nbody without close", "broken").is_none());
    }

    #[test]
    fn tolerates_bom_and_crlf() {
        let content = "\u{feff}---\r\non: github_pr\r\nagentType: claude-code\r\n---\r\n\r\nBody here.\r\n";
        let loop_ = parse_loop(content, "bom").expect("bom+crlf parses");
        assert_eq!(loop_.frontmatter.on, "github_pr");
        assert_eq!(loop_.frontmatter.agent_type.as_deref(), Some("claude-code"));
        assert_eq!(loop_.prompt_body, "Body here.");
    }

    #[test]
    fn ignores_unknown_frontmatter_fields() {
        let content = "---\non: github_pr\nfutureField: 123\n---\nbody\n";
        let loop_ = parse_loop(content, "extra").expect("unknown fields ignored");
        assert_eq!(loop_.frontmatter.on, "github_pr");
    }

    #[test]
    fn empty_body_is_allowed() {
        let content = "---\non: github_pr\n---\n";
        let loop_ = parse_loop(content, "no-body").expect("loop with empty body parses");
        assert_eq!(loop_.prompt_body, "");
    }
}
