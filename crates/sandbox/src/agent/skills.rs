//! Frontload activated skills into the user turn.
//!
//! The backend ships skill *names*; the body lives on this pod's disk. Each
//! name is looked up in order: the repo's own skill roots under every checkout,
//! then the harness's home skills dir, then each installed plugin's skills dir
//! — plugins keep their own root since nothing copies them into the home dir.
//!
//! The first root holding `<name>/SKILL.md` wins and names itself in the
//! preamble, so the agent knows which copy it was handed. Repo before plugin
//! matches how the backend merges the two for the picker.

use std::path::{Path, PathBuf};

use tracing::{debug, warn};

use crate::agent::handle::{AGENT_TYPE_CODEX, resolve_agent_harness};
use crate::config::Config;
use crate::init::home_skills::REPO_SKILL_ROOTS;

/// One place a skill can live, plus how the preamble names it.
struct SkillRoot {
    dir: PathBuf,
    /// Prefix the matched skill's directory is reported under. Repo-relative
    /// for the repo roots, absolute for the home dir — which no repo is
    /// relative to.
    label_prefix: String,
}

/// Prepend a preamble per resolved skill to `prompt`, in the order the backend
/// listed them.
///
/// A name that resolves nowhere is warned about and skipped; returns `prompt`
/// unchanged when nothing resolves.
pub(crate) async fn frontload_skills(config: &Config, skills: &[String], prompt: &str) -> String {
    if skills.is_empty() {
        return prompt.to_string();
    }

    let roots = skill_roots(config).await;
    let mut out = String::new();
    for name in skills {
        if !is_safe_skill_name(name) {
            warn!(skill = %name, "Skipping skill with an unusable name");
            continue;
        }
        if let Some((label, body)) = read_skill(&roots, name).await {
            debug!(skill = %name, label = %label, body_len = body.len(), "Activated skill");
            out.push_str(&preamble(&label, &body));
        } else {
            warn!(skill = %name, "Skill not found in any skill root; skipping");
        }
    }

    if out.is_empty() {
        return prompt.to_string();
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
    for name in config
        .plugins
        .iter()
        .flat_map(|plugin| plugin.frontload_skill_names.iter())
    {
        if !requested.contains(name) && !skills.contains(name) {
            skills.push(name.clone());
        }
    }
    skills.extend(requested);
    skills
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
            })
        })
        .collect();

    let home = home_skills_dir(config);
    roots.push(SkillRoot {
        label_prefix: home.display().to_string(),
        dir: home,
    });

    let plugins = config.plugins.clone();
    let plugin_dirs = tokio::task::spawn_blocking(move || {
        crate::init::plugin_skills::plugin_skill_dirs(&plugins)
    })
    .await
    .unwrap_or_else(|e| {
        warn!(error = %e, "Plugin skill scan failed; plugin skills will not resolve");
        Vec::new()
    });
    roots.extend(plugin_dirs.into_iter().map(|dir| SkillRoot {
        label_prefix: dir.display().to_string(),
        dir,
    }));
    roots
}

/// `<home>/skills` for the harness this Member is fixed to.
fn home_skills_dir(config: &Config) -> PathBuf {
    let is_codex = config
        .agent_type
        .as_deref()
        .and_then(|raw| resolve_agent_harness(raw).ok())
        == Some(AGENT_TYPE_CODEX);
    let home = if is_codex {
        &config.codex_home_dir
    } else {
        &config.claude_config_dir
    };
    Path::new(home).join("skills")
}

/// The first root holding `<name>/SKILL.md`, as `(label, body)`.
async fn read_skill(roots: &[SkillRoot], name: &str) -> Option<(String, String)> {
    for root in roots {
        let path = root.dir.join(name).join("SKILL.md");
        match tokio::fs::read_to_string(&path).await {
            Ok(body) => {
                return Some((format!("{}/{name}", root.label_prefix), body));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!(path = %path.display(), error = %e, "Failed to read SKILL.md"),
        }
    }
    None
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

/// A self-contained block ending in a blank line, so several concatenate and
/// the prompt follows without a separator of its own.
fn preamble(label: &str, body: &str) -> String {
    format!("Activated {label}:\n\n{}\n\n", body.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `Config` whose repo and home dirs point into `root`.
    fn config_in(root: &Path, agent_type: &str) -> Config {
        let mut config = crate::config::test_support::minimal_config();
        config.workspace_dir = root.join("workspace").display().to_string();
        config.repo_dir = root.join("workspace/repo").display().to_string();
        config.repos = Vec::new();
        config.claude_config_dir = root.join("home/.claude").display().to_string();
        config.codex_home_dir = root.join("home/.codex").display().to_string();
        config.agent_type = Some(agent_type.to_string());
        config
    }

    fn write_skill(dir: &Path, name: &str, body: &str) {
        let skill = dir.join(name);
        std::fs::create_dir_all(&skill).expect("create skill dir");
        std::fs::write(skill.join("SKILL.md"), body).expect("write SKILL.md");
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

    /// A name that resolves nowhere must not cost the turn.
    #[tokio::test]
    async fn an_unresolvable_skill_is_skipped() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = config_in(tmp.path(), "claude-code");
        write_skill(
            &tmp.path().join("workspace/repo/.agents/skills"),
            "real",
            "body",
        );

        let out = frontload_skills(&config, &["ghost".to_string(), "real".to_string()], "go").await;
        assert_eq!(out, "Activated .agents/skills/real:\n\nbody\n\ngo");
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

    // ---- Standing skills ----

    fn with_plugins(config: &mut Config, plugins: &[(&str, &[&str])]) {
        config.plugins = plugins
            .iter()
            .map(|(name, frontload)| moatless_types::plugin::PluginDelivery {
                name: (*name).to_string(),
                git_url: format!("https://example.test/{name}.git"),
                skill_names: frontload.iter().copied().map(String::from).collect(),
                frontload_skill_names: frontload.iter().copied().map(String::from).collect(),
            })
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
}
