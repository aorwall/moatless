//! SKILL.md frontmatter parser.
//!
//! Walks `{root}/.claude/skills/*/SKILL.md`, one directory deep, parses YAML
//! frontmatter into [`Skill`] rows. Individual parse failures are logged and
//! skipped so one malformed file doesn't tank the whole list.

use std::path::Path;

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

/// Scan `{root}/.claude/skills/` for SKILL.md files.
///
/// The returned skills are tagged with the provided `source` and optional
/// plugin metadata. If `.claude/skills/` doesn't exist the function returns
/// an empty vector — a repository without skills is not an error.
pub async fn scan_dir(
    root: &Path,
    source: SkillSource,
    plugin_repo_id: Option<&str>,
    plugin_repo_name: Option<&str>,
) -> std::io::Result<Vec<Skill>> {
    let skills_dir = root.join(".claude").join("skills");
    if !skills_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = tokio::fs::read_dir(&skills_dir).await?;
    let mut skills = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let file_type = match entry.file_type().await {
            Ok(ft) => ft,
            Err(e) => {
                warn!(path = %entry.path().display(), error = %e, "Failed to stat skill entry");
                continue;
            }
        };
        if !file_type.is_dir() {
            continue;
        }
        let skill_md = entry.path().join("SKILL.md");
        if !skill_md.exists() {
            continue;
        }

        let contents = match tokio::fs::read_to_string(&skill_md).await {
            Ok(c) => c,
            Err(e) => {
                warn!(path = %skill_md.display(), error = %e, "Failed to read SKILL.md");
                continue;
            }
        };

        let dir_name = entry
            .file_name()
            .to_string_lossy()
            .to_string();

        match parse_frontmatter(&contents) {
            Ok(fm) => {
                let name = fm.name.unwrap_or_else(|| dir_name.clone());
                let Some(description) = fm.description else {
                    warn!(path = %skill_md.display(), "Skipping skill: missing description");
                    continue;
                };
                skills.push(Skill {
                    name,
                    description,
                    source,
                    plugin_repo_id: plugin_repo_id.map(String::from),
                    plugin_repo_name: plugin_repo_name.map(String::from),
                });
            }
            Err(e) => {
                warn!(path = %skill_md.display(), error = %e, "Skipping skill: malformed frontmatter");
            }
        }
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
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
    use tempfile::TempDir;
    use tokio::fs;

    async fn write_skill(root: &Path, name: &str, body: &str) {
        let dir = root.join(".claude").join("skills").join(name);
        fs::create_dir_all(&dir).await.unwrap();
        fs::write(dir.join("SKILL.md"), body).await.unwrap();
    }

    #[tokio::test]
    async fn parses_valid_skill() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "backend",
            "---\nname: backend\ndescription: Backend dev patterns\n---\n\n# Body\n",
        )
        .await;

        let skills = scan_dir(tmp.path(), SkillSource::Workspace, None, None)
            .await
            .unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "backend");
        assert_eq!(skills[0].description, "Backend dev patterns");
        assert_eq!(skills[0].source, SkillSource::Workspace);
    }

    #[tokio::test]
    async fn falls_back_to_directory_name_when_name_missing() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "my-skill",
            "---\ndescription: No explicit name here\n---\n",
        )
        .await;

        let skills = scan_dir(tmp.path(), SkillSource::Workspace, None, None)
            .await
            .unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my-skill");
    }

    #[tokio::test]
    async fn skips_skill_missing_description() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "broken", "---\nname: broken\n---\n").await;

        let skills = scan_dir(tmp.path(), SkillSource::Workspace, None, None)
            .await
            .unwrap();
        assert!(skills.is_empty());
    }

    #[tokio::test]
    async fn skips_skill_without_frontmatter() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "no-fm",
            "# Just a markdown file with no frontmatter\n",
        )
        .await;

        let skills = scan_dir(tmp.path(), SkillSource::Workspace, None, None)
            .await
            .unwrap();
        assert!(skills.is_empty());
    }

    #[tokio::test]
    async fn returns_empty_when_skills_dir_missing() {
        let tmp = TempDir::new().unwrap();
        let skills = scan_dir(tmp.path(), SkillSource::Workspace, None, None)
            .await
            .unwrap();
        assert!(skills.is_empty());
    }

    #[tokio::test]
    async fn tags_plugin_metadata() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "deploy",
            "---\nname: deploy\ndescription: Deploy stuff\n---\n",
        )
        .await;

        let skills = scan_dir(
            tmp.path(),
            SkillSource::Plugin,
            Some("plugin-123"),
            Some("ops-plugin"),
        )
        .await
        .unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].source, SkillSource::Plugin);
        assert_eq!(skills[0].plugin_repo_id.as_deref(), Some("plugin-123"));
        assert_eq!(skills[0].plugin_repo_name.as_deref(), Some("ops-plugin"));
    }

    #[tokio::test]
    async fn scans_multiple_skills_sorted_by_name() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "zebra",
            "---\nname: zebra\ndescription: Z\n---\n",
        )
        .await;
        write_skill(
            tmp.path(),
            "alpha",
            "---\nname: alpha\ndescription: A\n---\n",
        )
        .await;
        write_skill(
            tmp.path(),
            "middle",
            "---\nname: middle\ndescription: M\n---\n",
        )
        .await;

        let skills = scan_dir(tmp.path(), SkillSource::Workspace, None, None)
            .await
            .unwrap();
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "middle", "zebra"]);
    }

    #[tokio::test]
    async fn ignores_non_directory_entries_in_skills_dir() {
        let tmp = TempDir::new().unwrap();
        let skills_root = tmp.path().join(".claude").join("skills");
        fs::create_dir_all(&skills_root).await.unwrap();
        fs::write(skills_root.join("stray.txt"), "not a skill").await.unwrap();
        write_skill(
            tmp.path(),
            "real",
            "---\nname: real\ndescription: Real skill\n---\n",
        )
        .await;

        let skills = scan_dir(tmp.path(), SkillSource::Workspace, None, None)
            .await
            .unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "real");
    }

    #[tokio::test]
    async fn ignores_extra_frontmatter_fields() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "extra",
            "---\nname: extra\ndescription: Has extras\nallowed-tools: Bash(git:*)\ncolor: red\n---\n",
        )
        .await;

        let skills = scan_dir(tmp.path(), SkillSource::Workspace, None, None)
            .await
            .unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "extra");
    }
}
