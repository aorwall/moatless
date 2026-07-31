//! `LOOP.md` frontmatter + body parser.
//!
//! Mirrors [`crate::skill::scanner`] but keeps the markdown body after the
//! closing `---` as the loop's prompt. Individual parse failures are returned
//! as an `Err` reason so one malformed file doesn't tank the whole sync — and
//! so the author is told which field is wrong instead of "malformed".

use super::models::{LoopFrontmatter, ParsedLoop};

/// Parse a `LOOP.md` body into a [`ParsedLoop`].
///
/// Returns `Err(reason)` when the frontmatter is missing, malformed, or omits a
/// required field (`on`, `name`). `dir_name` is the loop's stable sync key; the
/// name it is displayed under comes from the frontmatter.
pub fn parse_loop(content: &str, dir_name: &str) -> Result<ParsedLoop, String> {
    let (yaml, body) = split_frontmatter(content)?;

    let mut frontmatter: LoopFrontmatter =
        serde_yaml::from_str(&yaml).map_err(|e| format!("invalid frontmatter: {e}"))?;

    if frontmatter.on.trim().is_empty() {
        return Err("missing `on` field".to_string());
    }
    frontmatter.name = frontmatter.name.trim().to_string();
    if frontmatter.name.is_empty() {
        return Err("missing `name` field".to_string());
    }

    Ok(ParsedLoop {
        dir_name: dir_name.to_string(),
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
        let content =
            "---\non: github_pr\nname: Java Review\ntags: [review]\n---\n\nReview this PR.\n";
        let loop_ = parse_loop(content, "java-review").expect("valid loop parses");
        assert_eq!(loop_.dir_name, "java-review");
        assert_eq!(loop_.frontmatter.name, "Java Review");
        assert_eq!(loop_.frontmatter.on, "github_pr");
        assert_eq!(
            loop_.frontmatter.tags.as_deref(),
            Some(&["review".to_string()][..])
        );
        assert_eq!(loop_.prompt_body, "Review this PR.");
        // Absent inactiveTtlDays inherits the platform default (None).
        assert_eq!(loop_.frontmatter.inactive_ttl_days, None);
    }

    #[test]
    fn parses_inactive_ttl_days() {
        let content = "---\non: github_pr\nname: Java Review\ninactiveTtlDays: 1\n---\n\nbody\n";
        let loop_ = parse_loop(content, "java-review").expect("valid loop parses");
        assert_eq!(loop_.frontmatter.inactive_ttl_days, Some(1));
    }

    #[test]
    fn parses_schedule_loop() {
        let content = "---\non: schedule\nname: Daily Health Check\nschedule:\n  cron: \"0 0 6 * * MON-FRI *\"\n  timezone: Europe/Stockholm\n  message: \"Run the health check.\"\n---\n\nRun the daily-health-check skill.\n";
        let loop_ = parse_loop(content, "daily-health-check").expect("schedule loop parses");
        assert_eq!(loop_.frontmatter.on, "schedule");
        let sched = loop_.frontmatter.schedule.as_ref().expect("schedule block");
        assert_eq!(sched.cron.as_deref(), Some("0 0 6 * * MON-FRI *"));
        assert_eq!(sched.timezone.as_deref(), Some("Europe/Stockholm"));
        assert_eq!(sched.message.as_deref(), Some("Run the health check."));
        assert_eq!(loop_.prompt_body, "Run the daily-health-check skill.");
    }

    #[test]
    fn parses_slack_loop() {
        let content = "---\non: slack\nname: E2E Triage\nsourceMatcher: C0000000001\nteamId: T0000000001\nroutingMode: ongoing\nadapterConfig:\n  allow_bot_messages: true\n  trigger: all_messages\n---\n\nMonitor CI failures and triage them.\n";
        let loop_ = parse_loop(content, "e2e-triage").expect("slack loop parses");
        assert_eq!(loop_.frontmatter.on, "slack");
        assert_eq!(
            loop_.frontmatter.source_matcher.as_deref(),
            Some("C0000000001")
        );
        assert_eq!(loop_.frontmatter.team_id.as_deref(), Some("T0000000001"));
        assert_eq!(loop_.frontmatter.routing_mode.as_deref(), Some("ongoing"));
        let cfg = loop_
            .frontmatter
            .adapter_config
            .as_ref()
            .expect("adapterConfig present");
        assert_eq!(
            cfg.get("allow_bot_messages")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            cfg.get("trigger").and_then(serde_json::Value::as_str),
            Some("all_messages")
        );
        assert_eq!(loop_.prompt_body, "Monitor CI failures and triage them.");
    }

    #[test]
    fn parses_linear_loop() {
        let content = "---\non: linear\nname: Ready Pool\nsourceMatcher: team-uuid-123\ntriggerStates: [Ready]\nprojectIds: [proj-abc]\nrequireLabels: [agent]\n---\n\nPick up the ticket and implement it.\n";
        let loop_ = parse_loop(content, "ready-pool").expect("linear loop parses");
        assert_eq!(loop_.frontmatter.on, "linear");
        assert_eq!(
            loop_.frontmatter.source_matcher.as_deref(),
            Some("team-uuid-123")
        );
        assert_eq!(
            loop_.frontmatter.trigger_states.as_deref(),
            Some(&["Ready".to_string()][..])
        );
        assert_eq!(
            loop_.frontmatter.project_ids.as_deref(),
            Some(&["proj-abc".to_string()][..])
        );
        assert_eq!(
            loop_.frontmatter.require_labels.as_deref(),
            Some(&["agent".to_string()][..])
        );
        assert_eq!(loop_.prompt_body, "Pick up the ticket and implement it.");
    }

    #[test]
    fn skips_loop_missing_on() {
        // `on` is required by serde — missing key fails deserialization.
        let content = "---\nname: No On\nagentType: claude-code\n---\n\nbody\n";
        assert!(parse_loop(content, "no-on").is_err());
    }

    #[test]
    fn skips_loop_missing_name() {
        // The directory name is the sync key, not a name somebody chose — a
        // Loop that never says what to call it does not sync.
        let content = "---\non: github_pr\n---\n\nbody\n";
        let reason = parse_loop(content, "no-name").expect_err("name is required");
        assert!(reason.contains("name"), "reason names the field: {reason}");
    }

    #[test]
    fn skips_loop_with_blank_name() {
        let content = "---\non: github_pr\nname: \"   \"\n---\n\nbody\n";
        assert_eq!(
            parse_loop(content, "blank-name").expect_err("blank name is no name"),
            "missing `name` field"
        );
    }

    #[test]
    fn trims_surrounding_whitespace_from_name() {
        let content = "---\non: github_pr\nname: \"  Java Review  \"\n---\n\nbody\n";
        let loop_ = parse_loop(content, "java-review").expect("valid loop parses");
        assert_eq!(loop_.frontmatter.name, "Java Review");
    }

    #[test]
    fn skips_loop_without_frontmatter() {
        assert!(parse_loop("# just markdown\n", "no-fm").is_err());
    }

    #[test]
    fn skips_loop_with_unclosed_frontmatter() {
        assert!(parse_loop("---\non: github_pr\nbody without close", "broken").is_err());
    }

    #[test]
    fn tolerates_bom_and_crlf() {
        let content = "\u{feff}---\r\non: github_pr\r\nname: BOM Loop\r\nagentType: claude-code\r\n---\r\n\r\nBody here.\r\n";
        let loop_ = parse_loop(content, "bom").expect("bom+crlf parses");
        assert_eq!(loop_.frontmatter.on, "github_pr");
        assert_eq!(loop_.frontmatter.name, "BOM Loop");
        assert_eq!(loop_.frontmatter.agent_type.as_deref(), Some("claude-code"));
        assert_eq!(loop_.prompt_body, "Body here.");
    }

    #[test]
    fn ignores_unknown_frontmatter_fields() {
        let content = "---\non: github_pr\nname: Extra\nfutureField: 123\n---\nbody\n";
        let loop_ = parse_loop(content, "extra").expect("unknown fields ignored");
        assert_eq!(loop_.frontmatter.on, "github_pr");
    }

    /// Every `LOOP.md` this repository commits, parsed by the code the backend
    /// runs on it.
    ///
    /// A committed loop that does not parse is not an error anybody sees: sync
    /// logs a warning and moves on, and the loop simply never fires. This is
    /// where that shows up instead.
    #[test]
    fn the_repositorys_own_loops_parse() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.moatless/loops");
        let mut found = Vec::new();

        for entry in std::fs::read_dir(&root).expect("the repository has a .moatless/loops") {
            let dir = entry.expect("readable directory entry").path();
            let file = dir.join("LOOP.md");
            if !file.is_file() {
                continue;
            }
            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .expect("loop directory name")
                .to_string();
            let content = std::fs::read_to_string(&file).expect("readable LOOP.md");
            let parsed = parse_loop(&content, &name).unwrap_or_else(|reason| {
                panic!("{name}/LOOP.md does not parse ({reason}) — the loop never syncs")
            });
            assert!(
                !parsed.prompt_body.trim().is_empty(),
                "{name}/LOOP.md has no body, so the agent it spawns is told nothing"
            );
            found.push((name, parsed));
        }

        assert!(!found.is_empty(), "no loops found under {}", root.display());

        for (name, parsed) in &found {
            if parsed.frontmatter.on.trim() != "schedule" {
                continue;
            }
            let expression = parsed
                .frontmatter
                .schedule
                .as_ref()
                .and_then(|s| s.cron.as_deref())
                .unwrap_or_else(|| panic!("{name}: a schedule loop without schedule.cron is skipped by sync"));
            let schedule: cron::Schedule = expression
                .parse()
                .unwrap_or_else(|e| panic!("{name}: cron '{expression}' is invalid: {e}"));
            let mut fires = schedule.upcoming(chrono::Utc);
            let first = fires.next().expect("a cron with a future fire time");
            let second = fires.next().expect("a cron that fires more than once");
            // Each fire spawns a task that provisions a sandbox and runs a suite
            // for half an hour. A schedule that repeats faster than the work
            // takes is how a cluster fills with tasks nobody asked for.
            assert!(
                second - first >= chrono::Duration::hours(1),
                "{name}: cron '{expression}' fires every {} — too often for a loop that spawns a task each time",
                second - first
            );
        }
    }

    #[test]
    fn empty_body_is_allowed() {
        let content = "---\non: github_pr\nname: No Body\n---\n";
        let loop_ = parse_loop(content, "no-body").expect("loop with empty body parses");
        assert_eq!(loop_.prompt_body, "");
    }
}
