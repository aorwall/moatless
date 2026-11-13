//! Discover managed skills and frontload activated bodies into the user turn.
//!
//! The backend ships skill *names*; the body lives on this pod's disk. Each
//! name is looked up in order: the repo's own skill roots under every checkout,
//! then the harness's home skills dir, then each installed plugin's skills dir
//! — plugins keep their own root since nothing copies them into the home dir.
//!
//! The first root holding `<name>/SKILL.md` wins and names itself in the
//! preamble, so the agent knows which copy it was handed. Repo before plugin
//! matches how the backend merges the two for the picker.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tracing::{debug, warn};

use crate::agent::handle::resolve_agent_harness;
use crate::config::Config;
use moatless_workspace_types::plugin::REPO_SKILL_ROOTS;

/// One place a skill can live, plus how the preamble names it.
struct SkillRoot {
    dir: PathBuf,
    /// Prefix the matched skill's directory is reported under. Repo-relative
    /// for the repo roots, absolute for the home dir — which no repo is
    /// relative to.
    label_prefix: String,
    /// The plugin component a plugin root belongs to, whose path a skill body
    /// read from it gets for `${CLAUDE_PLUGIN_ROOT}`; `None` for the repo and
    /// home roots.
    plugin_root: Option<PathBuf>,
}

impl SkillRoot {
    /// How a preamble names skill `name` from this root.
    fn label(&self, name: &str) -> String {
        format!("{}/{name}", self.label_prefix)
    }
}

/// Prepend each of the turn's skills to `prompt` as its `SKILL.md` body, in the
/// order the backend listed them and each name once, and list a name that
/// resolves nowhere in a notice.
pub(crate) async fn frontload_skills(config: &Config, skills: &[String], prompt: &str) -> String {
    if skills.is_empty() {
        return prompt.to_string();
    }

    let roots = skill_roots(config).await;
    let mut seen = HashSet::new();
    let mut out = String::new();
    let mut missing = Vec::new();
    for name in skills.iter().filter(|name| seen.insert(*name)) {
        if !is_safe_skill_name(name) {
            warn!(skill = %name, "Skipping skill with an unusable name");
            continue;
        }
        let Some(root) = find_skill(&roots, name).await else {
            warn!(skill = %name, "Skill not found in any skill root; skipping");
            missing.push(format!("`{name}`"));
            continue;
        };
        if let Some((label, body)) = read_skill_in(root, name).await {
            debug!(skill = %name, label = %label, body_len = body.len(), "Activated skill");
            out.push_str(&preamble(&label, &body));
        }
    }
    // Stays empty for the dispatcher's empty-prompt check in
    // `dispatch_pending_messages_for_context`.
    if out.is_empty() && prompt.is_empty() {
        return String::new();
    }
    if !missing.is_empty() {
        out.push_str("Not installed in this sandbox, so not loaded: ");
        out.push_str(&missing.join(", "));
        out.push_str(". Tell the user these skills are missing before you work without them.\n\n");
    }
    out.push_str(prompt);
    out
}

/// The turn's own skills, behind the ones an activation record asked to
/// frontload everywhere.
///
/// The standing skills lead: what somebody picked for this turn belongs closest
/// to what they wrote. A name already requested is not repeated, nor one two
/// plugins both deliver.
pub(crate) fn with_always_frontloaded(config: &Config, requested: Vec<String>) -> Vec<String> {
    let mut skills: Vec<String> = Vec::new();
    for name in standing_skill_names(config) {
        if !requested.contains(name) && !skills.contains(name) {
            skills.push(name.clone());
        }
    }
    skills.extend(requested);
    skills
}

/// The marker Claude writes into a transcript where a compaction cut it.
const COMPACT_BOUNDARY: &str = r#""subtype":"compact_boundary""#;

fn standing_skill_names(config: &Config) -> impl Iterator<Item = &String> {
    config
        .plugins
        .iter()
        .flat_map(|plugin| plugin.frontload_skill_names.iter())
}

/// `skills` without those whose preamble `conversation` holds past its last
/// Claude compaction, for a turn that continues a session without a resume id.
/// `conversation` is the session's raw text; a label holds no JSON escapes.
pub(crate) async fn not_loaded_in(
    config: &Config,
    skills: Vec<String>,
    conversation: &str,
) -> Vec<String> {
    // The transcript keeps what a compaction summarised away from the model.
    let conversation = conversation
        .rfind(COMPACT_BOUNDARY)
        .map_or(conversation, |at| &conversation[at..]);
    let roots = skill_roots(config).await;
    let mut left = Vec::new();
    for name in skills {
        let loaded = match find_skill(&roots, &name).await {
            Some(root) if is_safe_skill_name(&name) => {
                conversation.contains(&format!("Activated {}:", root.label(&name)))
            }
            _ => false,
        };
        if !loaded {
            left.push(name);
        }
    }
    left
}

/// The roots to search, in precedence order.
///
/// The plugin scan reads manifests off disk, so it runs on a blocking worker;
/// the repo and home roots are pure path joins.
async fn skill_roots(config: &Config) -> Vec<SkillRoot> {
    let mut roots: Vec<SkillRoot> = config
        .repo_dirs()
        .iter()
        .flat_map(|repo_dir| {
            REPO_SKILL_ROOTS.iter().map(move |root| SkillRoot {
                dir: Path::new(repo_dir).join(root),
                label_prefix: (*root).to_string(),
                plugin_root: None,
            })
        })
        .collect();

    let home = home_skills_dir(config);
    roots.push(SkillRoot {
        label_prefix: home.display().to_string(),
        dir: home,
        plugin_root: None,
    });

    let plugins = config.plugins.clone();
    let plugins_dir = config.plugins_dir();
    let plugin_dirs = tokio::task::spawn_blocking(move || {
        crate::init::plugin_skills::plugin_skill_dirs(&plugins_dir, &plugins)
    })
    .await
    .unwrap_or_else(|e| {
        warn!(error = %e, "Plugin skill scan failed; plugin skills will not resolve");
        Vec::new()
    });
    roots.extend(plugin_dirs.into_iter().map(|dir| SkillRoot {
        label_prefix: dir.skills_dir.display().to_string(),
        dir: dir.skills_dir,
        plugin_root: Some(dir.plugin_root),
    }));
    roots
}

/// Managed skill roots passed explicitly to agents whose automatic discovery is disabled.
pub(crate) async fn discovery_paths(config: &Config) -> Vec<PathBuf> {
    skill_roots(config)
        .await
        .into_iter()
        .map(|root| root.dir)
        .collect()
}

/// Skill file paths and frontmatter descriptions for agents without native discovery.
pub(crate) async fn catalog_prompt(config: &Config) -> String {
    let roots = skill_roots(config).await;
    tokio::task::spawn_blocking(move || catalog_from_roots(&roots))
        .await
        .unwrap_or_else(|error| {
            warn!(%error, "Skill catalog scan failed");
            String::new()
        })
}

fn catalog_from_roots(roots: &[SkillRoot]) -> String {
    use std::io::Read;

    let mut seen = std::collections::HashSet::new();
    let mut rows = Vec::new();
    for root in roots {
        let entries = match std::fs::read_dir(&root.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                warn!(path = %root.dir.display(), %error, "Cannot scan skill root");
                continue;
            }
        };
        let mut paths: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("SKILL.md"))
            .collect();
        paths.sort();
        for path in paths {
            let Ok(canonical) = std::fs::canonicalize(&path) else {
                continue;
            };
            if !seen.insert(canonical.clone()) {
                continue;
            }
            let mut bytes = Vec::new();
            let read = std::fs::File::open(&canonical)
                .and_then(|file| file.take(64 * 1024).read_to_end(&mut bytes));
            if let Err(error) = read {
                warn!(path = %path.display(), %error, "Cannot read skill metadata");
                continue;
            }
            let contents = String::from_utf8_lossy(&bytes);
            let Some((yaml, _)) = split_frontmatter(&contents) else {
                continue;
            };
            let metadata = match serde_yaml::from_str::<serde_yaml::Value>(yaml) {
                Ok(metadata) => metadata,
                Err(error) => {
                    warn!(path = %path.display(), %error, "Invalid skill metadata");
                    continue;
                }
            };
            let Some(description) = metadata
                .get("description")
                .and_then(serde_yaml::Value::as_str)
                .filter(|description| !description.trim().is_empty())
            else {
                continue;
            };
            let name = metadata
                .get("name")
                .and_then(serde_yaml::Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .or_else(|| path.parent()?.file_name()?.to_str())
                .unwrap_or("skill");
            let mut row = serde_json::json!({"name": name, "description": description, "path": path.display().to_string()});
            if let Some(plugin_root) = &root.plugin_root {
                row["plugin_root"] = serde_json::json!(plugin_root.display().to_string());
            }
            rows.push(row.to_string());
        }
    }
    if rows.is_empty() {
        return String::new();
    }
    format!(
        "Available skills (one JSON object per skill):\nWhen the task matches a skill description, read its SKILL.md with bash before acting and follow its applicable instructions. Load only relevant skills and referenced resources. Resolve relative references from the directory containing SKILL.md. For plugin skills, substitute the listed plugin_root for ${{CLAUDE_PLUGIN_ROOT}} and the skill directory for ${{CLAUDE_SKILL_DIR}}.\n{}",
        rows.join("\n")
    )
}

/// `<home>/skills` for the harness this Member is fixed to.
fn home_skills_dir(config: &Config) -> PathBuf {
    let is_codex = config
        .agent_profile
        .as_deref()
        .and_then(|raw| resolve_agent_harness(raw).ok())
        == Some(moatless_types::agent::Harness::Codex);
    let home = if is_codex {
        &config.codex_home_dir
    } else {
        &config.claude_config_dir
    };
    Path::new(home).join("skills")
}

/// The first root holding `<name>/SKILL.md`.
async fn find_skill<'a>(roots: &'a [SkillRoot], name: &str) -> Option<&'a SkillRoot> {
    for root in roots {
        let path = root.dir.join(name).join("SKILL.md");
        match tokio::fs::try_exists(&path).await {
            Ok(true) => return Some(root),
            Ok(false) => {}
            Err(e) => warn!(path = %path.display(), error = %e, "Failed to stat SKILL.md"),
        }
    }
    None
}

/// `root`'s `<name>/SKILL.md` past its frontmatter (see [`strip_frontmatter`]),
/// with a plugin skill's placeholders expanded.
async fn read_skill_in(root: &SkillRoot, name: &str) -> Option<(String, String)> {
    let skill_dir = root.dir.join(name);
    let path = skill_dir.join("SKILL.md");
    let contents = match tokio::fs::read_to_string(&path).await {
        Ok(contents) => contents,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Failed to read SKILL.md");
            return None;
        }
    };
    let body = strip_frontmatter(&contents);
    let body = match &root.plugin_root {
        Some(plugin_root) => {
            crate::init::plugin_skills::expand_plugin_placeholders(body, plugin_root, &skill_dir)
        }
        None => body.to_string(),
    };
    Some((root.label(name), body))
}

/// Reject anything that is not a single path segment: the name is joined onto a
/// root, and `..` or a separator would read outside it.
fn is_safe_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

/// A SKILL.md past a leading YAML frontmatter block, or the whole file when
/// it opens with anything else. Frontmatter configures the harness, so in a
/// prompt it reads as an instruction to the agent. Only a first-line block
/// closed by a `---` line goes: a horizontal rule further down survives, and
/// an unclosed block is left alone rather than swallowing the body.
fn strip_frontmatter(contents: &str) -> &str {
    split_frontmatter(contents).map_or(contents, |(_, body)| body)
}

fn split_frontmatter(contents: &str) -> Option<(&str, &str)> {
    let trimmed = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    let rest = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))?;
    let mut cursor = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']) == "---" {
            return Some((
                &rest[..cursor],
                rest[cursor + line.len()..].trim_start_matches(['\n', '\r']),
            ));
        }
        cursor += line.len();
    }
    None
}

/// A self-contained block ending in a blank line, so several concatenate and
/// the prompt follows without a separator of its own.
fn preamble(label: &str, body: &str) -> String {
    format!("Activated {label}:\n\n{}\n\n", body.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `Config` whose repo and home dirs point into `root`.
    fn config_in(root: &Path, agent_profile: &str) -> Config {
        let mut config = crate::config::test_support::minimal_config();
        config.workspace_dir = root.join("workspace").display().to_string();
        config.repo_dir = root.join("workspace/repo").display().to_string();
        config.repos = Vec::new();
        config.claude_config_dir = root.join("home/.claude").display().to_string();
        config.codex_home_dir = root.join("home/.codex").display().to_string();
        config.agent_profile = Some(agent_profile.to_string());
        config
    }

    fn write_skill(dir: &Path, name: &str, body: &str) {
        let skill = dir.join(name);
        std::fs::create_dir_all(&skill).expect("create skill dir");
        std::fs::write(skill.join("SKILL.md"), body).expect("write SKILL.md");
    }

    #[tokio::test]
    async fn catalog_lists_yaml_metadata_and_paths_without_loading_skill_bodies() {
        let tmp = tempfile::tempdir().unwrap();
        let config = config_in(tmp.path(), "mini-swe");
        let root = Path::new(&config.repo_dir).join(".agents/skills");
        write_skill(
            &root,
            "release",
            "\u{feff}---\r\nname: Ship it\r\ndescription: >-\r\n  Prepare a release\r\n  and verify its artifacts.\r\n---\r\nBODY_ONLY_SECRET\n",
        );
        write_skill(
            &root,
            "invalid",
            "---\nname: [broken\ndescription: broken\n---\n",
        );
        write_skill(&root, "missing", "---\nname: Missing description\n---\n");
        write_skill(
            &Path::new(&config.claude_config_dir).join("skills"),
            "home-skill",
            "---\ndescription: 'Use for home tasks: verify output.'\n---\nHOME_BODY\n",
        );

        let prompt = catalog_prompt(&config).await;
        let rows: Vec<serde_json::Value> = prompt
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "Ship it");
        assert_eq!(
            rows[0]["description"],
            "Prepare a release and verify its artifacts."
        );
        assert_eq!(
            rows[0]["path"],
            root.join("release/SKILL.md").display().to_string()
        );
        assert_eq!(rows[1]["name"], "home-skill");
        assert!(!prompt.contains("BODY_ONLY_SECRET"));
        assert!(!prompt.contains("HOME_BODY"));
    }

    #[cfg(unix)]
    #[test]
    fn catalog_deduplicates_symlinks_and_preserves_plugin_reference_context() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin = tmp.path().join("plugin");
        let skills = plugin.join("skills");
        write_skill(
            &skills,
            "review",
            "---\nname: review\ndescription: Review code\n---\nRead ${CLAUDE_PLUGIN_ROOT}/rules.md\n",
        );
        let alias = tmp.path().join("alias");
        std::os::unix::fs::symlink(&skills, &alias).unwrap();
        let roots = [skills, alias].map(|dir| SkillRoot {
            label_prefix: String::new(),
            dir,
            plugin_root: Some(plugin.clone()),
        });
        let prompt = catalog_from_roots(&roots);
        let rows: Vec<serde_json::Value> = prompt
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["plugin_root"], plugin.display().to_string());
        assert!(prompt.contains("${CLAUDE_SKILL_DIR}"));
    }

    #[tokio::test]
    async fn an_empty_skill_catalog_adds_no_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            catalog_prompt(&config_in(tmp.path(), "mini-swe"))
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn no_skills_leaves_the_prompt_untouched() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");
        assert_eq!(
            frontload_skills(&config, &[], "do the thing").await,
            "do the thing"
        );
    }

    #[tokio::test]
    async fn a_repo_skill_is_frontloaded_under_its_repo_relative_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");
        write_skill(
            &tmp.path().join("workspace/repo/.agents/skills"),
            "e2e-testing",
            "# E2E\nRun the suite.",
        );

        let out = frontload_skills(&config, &["e2e-testing".to_string()], "go").await;
        assert_eq!(
            out,
            "Activated .agents/skills/e2e-testing:\n\n# E2E\nRun the suite.\n\ngo"
        );
    }

    /// Harness keys must not reach the prompt: an agent handed
    /// `disable-model-invocation: true` reads it as a rule forbidding the work
    /// the body describes.
    #[tokio::test]
    async fn frontmatter_is_stripped_from_the_frontloaded_body() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");
        write_skill(
            &tmp.path().join("workspace/repo/.agents/skills"),
            "pr-review",
            "---\nname: pr-review\ndescription: The PR review workflow\n\
             disable-model-invocation: true\nallowed-tools: Bash(git:*)\n---\n\n\
             # PR Review Workflow\nReview it.",
        );

        let out = frontload_skills(&config, &["pr-review".to_string()], "go").await;
        assert_eq!(
            out,
            "Activated .agents/skills/pr-review:\n\n# PR Review Workflow\nReview it.\n\ngo"
        );
    }

    /// A `---` rule in the body is content, not a delimiter, so everything
    /// after it has to survive.
    #[tokio::test]
    async fn a_horizontal_rule_in_the_body_survives() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");
        write_skill(
            &tmp.path().join("workspace/repo/.agents/skills"),
            "ruled",
            "---\nname: ruled\n---\n\n# Steps\n\n---\n\n## More\nKeep this.",
        );

        let out = frontload_skills(&config, &["ruled".to_string()], "go").await;
        assert_eq!(
            out,
            "Activated .agents/skills/ruled:\n\n# Steps\n\n---\n\n## More\nKeep this.\n\ngo"
        );
    }

    #[tokio::test]
    async fn a_skill_without_frontmatter_is_frontloaded_unchanged() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");
        let skills = tmp.path().join("workspace/repo/.agents/skills");
        write_skill(&skills, "plain", "# Plain\n\n---\n\nStill here.");
        // An unclosed block is body too: with no closing `---` there is no way
        // to tell where the frontmatter would have ended.
        write_skill(&skills, "unclosed", "---\nname: unclosed\n\n# Body");

        let out = frontload_skills(&config, &["plain".to_string()], "go").await;
        assert_eq!(
            out,
            "Activated .agents/skills/plain:\n\n# Plain\n\n---\n\nStill here.\n\ngo"
        );

        let out = frontload_skills(&config, &["unclosed".to_string()], "go").await;
        assert_eq!(
            out,
            "Activated .agents/skills/unclosed:\n\n---\nname: unclosed\n\n# Body\n\ngo"
        );
    }

    /// Nothing but frontmatter leaves an empty body rather than a panic or a
    /// leaked key.
    #[tokio::test]
    async fn a_skill_that_is_only_frontmatter_frontloads_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");
        let skills = tmp.path().join("workspace/repo/.agents/skills");
        write_skill(
            &skills,
            "empty",
            "---\ndisable-model-invocation: true\n---\n",
        );
        // Same file without the final newline: the closing `---` is the last
        // line, so the slice past it must stay in bounds.
        write_skill(
            &skills,
            "unterminated",
            "---\ndisable-model-invocation: true\n---",
        );

        for name in ["empty", "unterminated"] {
            let out = frontload_skills(&config, &[name.to_string()], "go").await;
            assert_eq!(out, format!("Activated .agents/skills/{name}:\n\n\n\ngo"));
        }
    }

    /// Both roots are searched, and `.claude/skills` is the one that names the
    /// skill when a repo exposes it under both (typically as a symlink).
    #[tokio::test]
    async fn the_claude_root_wins_over_the_agents_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");
        let repo = tmp.path().join("workspace/repo");
        write_skill(&repo.join(".claude/skills"), "shared", "claude copy");
        write_skill(&repo.join(".agents/skills"), "shared", "agents copy");

        let out = frontload_skills(&config, &["shared".to_string()], "go").await;
        assert!(out.starts_with("Activated .claude/skills/shared:"), "{out}");
        assert!(out.contains("claude copy"), "{out}");
        assert!(!out.contains("agents copy"), "{out}");
    }

    /// Plugin and image-staged skills only exist in the home dir, so a name the
    /// repo does not carry falls through to it.
    #[tokio::test]
    async fn a_home_skill_is_found_when_the_repo_has_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");
        let home = tmp.path().join("home/.claude/skills");
        write_skill(&home, "from-plugin", "plugin body");

        let out = frontload_skills(&config, &["from-plugin".to_string()], "go").await;
        assert_eq!(
            out,
            format!(
                "Activated {}/from-plugin:\n\nplugin body\n\ngo",
                home.display()
            )
        );
    }

    #[tokio::test]
    async fn codex_reads_its_own_home_skills_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "codex");
        write_skill(
            &tmp.path().join("home/.claude/skills"),
            "picked",
            "claude body",
        );
        write_skill(
            &tmp.path().join("home/.codex/skills"),
            "picked",
            "codex body",
        );

        let out = frontload_skills(&config, &["picked".to_string()], "go").await;
        assert!(out.contains("codex body"), "{out}");
        assert!(!out.contains("claude body"), "{out}");
    }

    #[tokio::test]
    async fn several_skills_keep_the_order_they_were_asked_for() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");
        let repo_skills = tmp.path().join("workspace/repo/.agents/skills");
        write_skill(&repo_skills, "first", "one");
        write_skill(&repo_skills, "second", "two");

        let out =
            frontload_skills(&config, &["second".to_string(), "first".to_string()], "go").await;
        assert_eq!(
            out,
            "Activated .agents/skills/second:\n\ntwo\n\n\
             Activated .agents/skills/first:\n\none\n\ngo"
        );
    }

    /// A name that resolves nowhere must not cost the turn, but the agent has
    /// to learn its rules are absent.
    #[tokio::test]
    async fn an_unresolvable_skill_is_skipped_and_named() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");
        write_skill(
            &tmp.path().join("workspace/repo/.agents/skills"),
            "real",
            "body",
        );

        let out = frontload_skills(&config, &["ghost".to_string(), "real".to_string()], "go").await;
        assert_eq!(
            out,
            "Activated .agents/skills/real:\n\nbody\n\n\
             Not installed in this sandbox, so not loaded: `ghost`. Tell the user these \
             skills are missing before you work without them.\n\ngo"
        );
    }

    #[tokio::test]
    async fn a_skills_only_delivery_that_resolves_nothing_stays_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");

        let out = frontload_skills(&config, &["ghost".to_string()], "").await;
        assert_eq!(out, "");
    }

    #[tokio::test]
    async fn a_traversing_name_reads_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");
        write_skill(
            &tmp.path().join("workspace/repo/.agents"),
            "skills",
            "escaped",
        );

        for name in ["..", "../skills", "a/b", ""] {
            let out = frontload_skills(&config, &[name.to_string()], "go").await;
            assert_eq!(out, "go", "name {name:?} should resolve nothing");
        }
    }

    /// A frontloaded body bypasses Claude Code's Skill tool, which is what
    /// would otherwise expand the placeholders a plugin skill's commands use.
    #[tokio::test]
    async fn a_plugin_skill_body_names_its_plugin_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut config = config_in(tmp.path(), "claude-code");
        with_plugins(&mut config, &[("agentops", &[])]);
        let plugin = config.plugins_dir().join("agentops");
        write_skill(
            &plugin.join("skills"),
            "pr-review",
            "python3 \"${CLAUDE_PLUGIN_ROOT}/skills/pr-review/scripts/review-scope.py\"",
        );

        let out = frontload_skills(&config, &["pr-review".to_string()], "go").await;
        assert_eq!(
            out,
            format!(
                "Activated {skills}/pr-review:\n\npython3 \"{root}/skills/pr-review/scripts/review-scope.py\"\n\ngo",
                skills = plugin.join("skills").display(),
                root = plugin.display(),
            )
        );
    }

    // ---- Standing skills ----

    fn with_plugins(config: &mut Config, plugins: &[(&str, &[&str])]) {
        config.plugins = plugins
            .iter()
            .map(
                |(name, frontload)| moatless_workspace_types::plugin::PluginDelivery {
                    name: (*name).to_string(),
                    git_url: format!("https://example.test/{name}.git"),
                    skill_names: frontload.iter().copied().map(String::from).collect(),
                    frontload_skill_names: frontload.iter().copied().map(String::from).collect(),
                },
            )
            .collect();
    }

    #[test]
    fn a_standing_skill_leads_the_skills_the_turn_asked_for() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut config = config_in(tmp.path(), "claude-code");
        with_plugins(&mut config, &[("guidelines", &["house-style"])]);

        assert_eq!(
            with_always_frontloaded(&config, vec!["e2e-testing".to_string()]),
            vec!["house-style".to_string(), "e2e-testing".to_string()],
            "what somebody picked for the turn belongs closest to what they wrote"
        );
    }

    #[test]
    fn a_standing_skill_the_turn_also_asked_for_is_frontloaded_once() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut config = config_in(tmp.path(), "claude-code");
        with_plugins(
            &mut config,
            &[
                ("guidelines", &["house-style"]),
                ("extra", &["house-style"]),
            ],
        );

        assert_eq!(
            with_always_frontloaded(&config, vec!["house-style".to_string()]),
            vec!["house-style".to_string()],
            "a body repeated in the same prompt is cost without content"
        );
    }

    #[test]
    fn no_standing_skills_leaves_the_turn_as_it_was() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");

        assert_eq!(
            with_always_frontloaded(&config, vec!["e2e-testing".to_string()]),
            vec!["e2e-testing".to_string()]
        );
        assert!(with_always_frontloaded(&config, Vec::new()).is_empty());
    }

    #[tokio::test]
    async fn a_skill_the_conversation_already_holds_is_not_repeated() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut config = config_in(tmp.path(), "claude-code");
        with_plugins(
            &mut config,
            &[("guidelines", &["house-style", "e2e-testing"])],
        );
        let skills = tmp.path().join("workspace/repo/.agents/skills");
        write_skill(&skills, "house-style", "Write plainly.");
        write_skill(&skills, "e2e-testing", "Run the suite.");
        let first_prompt = frontload_skills(&config, &["house-style".to_string()], "hi").await;
        let conversation =
            serde_json::json!({ "message": { "content": first_prompt } }).to_string();

        let turn = with_always_frontloaded(&config, vec!["review".to_string()]);
        write_skill(&skills, "review", "Review it.");
        assert_eq!(
            not_loaded_in(&config, turn, &conversation).await,
            vec!["e2e-testing".to_string(), "review".to_string()]
        );
    }

    #[tokio::test]
    async fn a_skill_pasted_before_a_compaction_is_pasted_again() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");
        let skills = tmp.path().join("workspace/repo/.agents/skills");
        write_skill(&skills, "review", "Review it.");
        let first = frontload_skills(&config, &["review".to_string()], "hi").await;
        let pasted = serde_json::json!({ "message": { "content": first } }).to_string();
        let compacted = serde_json::json!({ "type": "system", "subtype": "compact_boundary" });

        let conversation = format!("{pasted}\n{compacted}");
        assert_eq!(
            not_loaded_in(&config, vec!["review".to_string()], &conversation).await,
            vec!["review".to_string()]
        );
        let conversation = format!("{compacted}\n{pasted}");
        assert!(
            not_loaded_in(&config, vec!["review".to_string()], &conversation)
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_skill_named_twice_in_one_turn_is_pasted_once() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");
        write_skill(
            &tmp.path().join("workspace/repo/.agents/skills"),
            "review",
            "Review it.",
        );

        let out =
            frontload_skills(&config, &["review".to_string(), "review".to_string()], "go").await;
        assert_eq!(out, "Activated .agents/skills/review:\n\nReview it.\n\ngo");
    }
}
