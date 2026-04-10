//! SKILL.md frontmatter parser.
//!
//! Given the raw body of a `SKILL.md` file and the directory name it came
//! from, returns a parsed [`Skill`] with the appropriate source metadata.
//! Individual parse failures are logged and returned as `None` so one
//! malformed file doesn't tank the whole list.

use serde::Deserialize;
use tracing::warn;

use super::models::{Skill, SkillSource};

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// Parse a SKILL.md body into a [`Skill`].
///
/// Returns `None` (after logging a warning) when the frontmatter is missing,
/// malformed, or has no `description:` field. `plugin_repo_id` and
/// `plugin_repo_name` are recorded verbatim on the returned skill.
pub fn parse_skill(
    content: &str,
    dir_name: &str,
    source: SkillSource,
    plugin_repo_id: Option<&str>,
    plugin_repo_name: Option<&str>,
) -> Option<Skill> {
    let fm = match parse_frontmatter(content) {
        Ok(fm) => fm,
        Err(e) => {
            warn!(dir_name, error = %e, "Skipping skill: malformed frontmatter");
            return None;
        }
    };

    let Some(description) = fm.description else {
        warn!(dir_name, "Skipping skill: missing description");
        return None;
    };

    let name = fm.name.unwrap_or_else(|| dir_name.to_string());
    Some(Skill {
        name,
        description,
        source,
        plugin_repo_id: plugin_repo_id.map(String::from),
        plugin_repo_name: plugin_repo_name.map(String::from),
    })
}

fn parse_frontmatter(contents: &str) -> Result<SkillFrontmatter, String> {
    // Strip the optional UTF-8 BOM so `---` lands at byte 0.
    let trimmed = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    // Frontmatter must start at the very first line.
    let rest = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))
        .ok_or_else(|| "missing opening '---' delimiter".to_string())?;

    // Find the closing delimiter. We look for a line that is exactly `---`.
    let mut end_byte = None;
    let mut cursor = 0usize;
    for line in rest.split_inclusive('\n') {
        let line_content = line.trim_end_matches(['\n', '\r']);
        if line_content == "---" {
            end_byte = Some(cursor);
            break;
        }
        cursor += line.len();
    }
    let end_byte = end_byte.ok_or_else(|| "missing closing '---' delimiter".to_string())?;
    let yaml = &rest[..end_byte];

    serde_yaml::from_str::<SkillFrontmatter>(yaml).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_skill() {
        let skill = parse_skill(
            "---\nname: backend\ndescription: Backend dev patterns\n---\n\n# Body\n",
            "backend",
            SkillSource::Workspace,
            None,
            None,
        )
        .expect("valid skill parses");
        assert_eq!(skill.name, "backend");
        assert_eq!(skill.description, "Backend dev patterns");
        assert_eq!(skill.source, SkillSource::Workspace);
    }

    #[test]
    fn falls_back_to_directory_name_when_name_missing() {
        let skill = parse_skill(
            "---\ndescription: No explicit name here\n---\n",
            "my-skill",
            SkillSource::Workspace,
            None,
            None,
        )
        .expect("skill without explicit name parses");
        assert_eq!(skill.name, "my-skill");
    }

    #[test]
    fn skips_skill_missing_description() {
        let skill = parse_skill(
            "---\nname: broken\n---\n",
            "broken",
            SkillSource::Workspace,
            None,
            None,
        );
        assert!(skill.is_none());
    }

    #[test]
    fn skips_skill_without_frontmatter() {
        let skill = parse_skill(
            "# Just a markdown file with no frontmatter\n",
            "no-fm",
            SkillSource::Workspace,
            None,
            None,
        );
        assert!(skill.is_none());
    }

    #[test]
    fn tags_plugin_metadata() {
        let skill = parse_skill(
            "---\nname: deploy\ndescription: Deploy stuff\n---\n",
            "deploy",
            SkillSource::Plugin,
            Some("plugin-123"),
            Some("ops-plugin"),
        )
        .expect("plugin skill parses");
        assert_eq!(skill.source, SkillSource::Plugin);
        assert_eq!(skill.plugin_repo_id.as_deref(), Some("plugin-123"));
        assert_eq!(skill.plugin_repo_name.as_deref(), Some("ops-plugin"));
    }

    #[test]
    fn ignores_extra_frontmatter_fields() {
        let skill = parse_skill(
            "---\nname: extra\ndescription: Has extras\nallowed-tools: Bash(git:*)\ncolor: red\n---\n",
            "extra",
            SkillSource::Workspace,
            None,
            None,
        )
        .expect("extra fields parse");
        assert_eq!(skill.name, "extra");
    }

    #[test]
    fn tolerates_bom_and_crlf() {
        let body = "\u{feff}---\r\nname: bom\r\ndescription: BOM handled\r\n---\r\n";
        let skill = parse_skill(body, "bom", SkillSource::Workspace, None, None)
            .expect("bom+crlf parses");
        assert_eq!(skill.name, "bom");
    }
}
