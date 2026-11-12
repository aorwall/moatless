//! Warn when a turn's text or a checkout's instructions name a plugin skill
//! this sandbox was not given.
//!
//! A plugin's activation records can pick a subset of its skills, and
//! `init::workspace::fetch_plugins` deletes the rest. A LOOP.md, a brief or an
//! `AGENTS.md` that names `factory:spec` then points at nothing, and the agent
//! only finds out when the Skill tool answers "Unknown skill".

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use tracing::warn;

use crate::config::Config;
use crate::init::plugin_skills::PluginNamespace;

/// The instruction files read at each checkout's root, beside the prompt.
const INSTRUCTION_FILES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// A notice naming each `namespace:skill` that `prompt` or a checkout's
/// instruction files reference, whose namespace a delivered plugin declares,
/// and whose `SKILL.md` is not on disk; `None` when there is none.
///
/// `noticed` holds the `namespace:skill` references this session was already
/// told about; each new one is added.
pub(crate) async fn unresolved_notice(
    config: &Config,
    prompt: &str,
    noticed: &std::sync::Mutex<HashSet<String>>,
) -> Option<String> {
    let plugins = config.plugins.clone();
    let plugins_dir = config.plugins_dir();
    let repo_dirs = config.repo_dirs();
    let prompt = prompt.to_string();
    let missing = tokio::task::spawn_blocking(move || {
        let namespaces = crate::init::plugin_skills::plugin_namespaces(&plugins_dir, &plugins);
        if namespaces.is_empty() {
            return Vec::new();
        }
        let mut sources = vec![("this message".to_string(), prompt)];
        sources.extend(instruction_files(&repo_dirs));
        unresolved(&namespaces, &sources)
    })
    .await
    .unwrap_or_else(|e| {
        warn!(error = %e, "Skill reference scan failed");
        Vec::new()
    });

    let mut missing = missing;
    {
        let mut noticed = noticed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        missing.retain(|m| noticed.insert(m.reference.clone()));
    }
    if missing.is_empty() {
        return None;
    }
    Some(notice(&missing))
}

/// One reference that resolves to no skill on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Missing {
    reference: String,
    source: String,
    plugin: String,
    git_url: String,
    skill_path: String,
}

/// Each checkout's root instruction files as `(label, text)`, a file reached
/// twice through a symlink read once.
fn instruction_files(repo_dirs: &[String]) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for repo_dir in repo_dirs {
        let repo = Path::new(repo_dir);
        let repo_name = repo
            .file_name()
            .map_or_else(|| repo_dir.clone(), |n| n.to_string_lossy().into_owned());
        for file in INSTRUCTION_FILES {
            let path = repo.join(file);
            let Ok(canonical) = std::fs::canonicalize(&path) else {
                continue;
            };
            if !seen.insert(canonical.clone()) {
                continue;
            }
            match std::fs::read_to_string(&canonical) {
                Ok(text) => files.push((format!("{file} in {repo_name}"), text)),
                Err(e) => warn!(path = %path.display(), error = %e, "Cannot read instructions"),
            }
        }
    }
    files
}

/// The references in `sources` that name a skill missing from its namespace,
/// first source first, each reference once.
fn unresolved(namespaces: &[PluginNamespace], sources: &[(String, String)]) -> Vec<Missing> {
    let by_name: BTreeMap<&str, &PluginNamespace> =
        namespaces.iter().map(|n| (n.name.as_str(), n)).collect();
    let mut seen = HashSet::new();
    let mut missing = Vec::new();
    for (source, text) in sources {
        for (namespace, skill) in references(text) {
            let Some(ns) = by_name.get(namespace) else {
                continue;
            };
            let reference = format!("{namespace}:{skill}");
            if !seen.insert(reference.clone()) {
                continue;
            }
            if ns.skills_dir.join(skill).join("SKILL.md").is_file() {
                continue;
            }
            missing.push(Missing {
                reference,
                source: source.clone(),
                plugin: ns.plugin.clone(),
                git_url: ns.git_url.clone(),
                skill_path: format!("{}/{skill}/SKILL.md", ns.repo_skills_dir),
            });
        }
    }
    missing
}

/// Every `namespace:skill` token in `text`, both halves lowercase kebab-case.
///
/// A token glued to a longer word, a path or a URL on either side is not one,
/// so `https://x` and `a/b:c` yield nothing.
fn references(text: &str) -> Vec<(&str, &str)> {
    fn is_part(c: char) -> bool {
        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
    }
    fn is_glue(c: char) -> bool {
        c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.' | ':' | '@' | '$')
    }
    let mut out = Vec::new();
    for (colon, _) in text.match_indices(':') {
        let before = &text[..colon];
        let ns_start = before
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_part(*c))
            .last()
            .map_or(colon, |(i, _)| i);
        let namespace = &text[ns_start..colon];
        if !namespace.starts_with(|c: char| c.is_ascii_lowercase())
            || before[..ns_start].chars().next_back().is_some_and(is_glue)
        {
            continue;
        }
        let after = &text[colon + 1..];
        let skill_len = after.find(|c: char| !is_part(c)).unwrap_or(after.len());
        let skill = after[..skill_len].trim_end_matches('-');
        let rest = &after[skill.len()..];
        let glued = rest.starts_with(|c: char| is_glue(c) && c != '.' && c != ':')
            || rest.starts_with(['.', ':']) && rest[1..].starts_with(|c: char| c.is_alphanumeric());
        if skill.starts_with(|c: char| c.is_ascii_lowercase()) && !glued {
            out.push((namespace, skill));
        }
    }
    out
}

fn notice(missing: &[Missing]) -> String {
    let mut out = String::from(
        "Named but not delivered to this sandbox: the plugin is installed, but its activation \
         records for this Workspace and run-as user leave these skills out, so the Skill tool \
         will not find them. Tell the user which are missing before you work without them; an \
         admin turns one on with `moat plugins enable`.\n",
    );
    for m in missing {
        let _ = writeln!(
            out,
            "- `{}`, named in {}: plugin `{}`, {} at {}",
            m.reference, m.source, m.plugin, m.git_url, m.skill_path
        );
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn namespace(root: &Path, name: &str) -> PluginNamespace {
        PluginNamespace {
            name: name.to_string(),
            plugin: "soaplabs".to_string(),
            git_url: "https://github.com/acme/skills".to_string(),
            repo_skills_dir: format!("plugins/{name}/skills"),
            skills_dir: root.join(name),
        }
    }

    fn with_skill(root: &Path, namespace: &str, skill: &str) {
        let dir = root.join(namespace).join(skill);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "body").unwrap();
    }

    #[test]
    fn references_are_namespaced_kebab_tokens_not_urls_or_paths() {
        let text = "Load `factory:spec`, then backend:database-migrations. \
                    See https://x.test and a/b:c, ws:8080, Foo:Bar, x-factory:spec-, \
                    re:fix: and (frontend:verify-soap-frontend).";
        assert_eq!(
            references(text),
            [
                ("factory", "spec"),
                ("backend", "database-migrations"),
                ("re", "fix"),
                ("frontend", "verify-soap-frontend"),
            ]
        );
    }

    #[test]
    fn a_reference_to_a_pruned_skill_is_reported_once_with_its_first_source() {
        let tmp = tempfile::tempdir().unwrap();
        with_skill(tmp.path(), "factory", "build");
        let namespaces = [namespace(tmp.path(), "factory")];
        let sources = [
            (
                "this message".to_string(),
                "Use factory:spec, then factory:build.".to_string(),
            ),
            ("AGENTS.md in repo".to_string(), "factory:spec".to_string()),
        ];

        let missing = unresolved(&namespaces, &sources);

        assert_eq!(
            missing,
            [Missing {
                reference: "factory:spec".to_string(),
                source: "this message".to_string(),
                plugin: "soaplabs".to_string(),
                git_url: "https://github.com/acme/skills".to_string(),
                skill_path: "plugins/factory/skills/spec/SKILL.md".to_string(),
            }]
        );
    }

    #[test]
    fn a_namespace_no_delivered_plugin_declares_is_not_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let namespaces = [namespace(tmp.path(), "factory")];
        let sources = [(
            "this message".to_string(),
            "re:fix and moat:cli".to_string(),
        )];
        assert!(unresolved(&namespaces, &sources).is_empty());
    }

    #[tokio::test]
    async fn the_notice_names_an_agents_md_reference_once_per_session() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = crate::config::test_support::minimal_config();
        config.workspace_dir = tmp.path().join("workspace").display().to_string();
        config.repo_dir = tmp.path().join("workspace/repo").display().to_string();
        config.repos = Vec::new();
        let plugin: PathBuf = config.plugins_dir().join("soaplabs");
        std::fs::create_dir_all(plugin.join("skills/kept")).unwrap();
        std::fs::write(plugin.join("skills/kept/SKILL.md"), "body").unwrap();
        config
            .plugins
            .push(moatless_workspace_types::plugin::PluginDelivery {
                name: "soaplabs".to_string(),
                git_url: "https://github.com/acme/skills".to_string(),
                skill_names: vec!["kept".to_string()],
                frontload_skill_names: Vec::new(),
            });
        std::fs::create_dir_all(&config.repo_dir).unwrap();
        std::fs::write(
            Path::new(&config.repo_dir).join("AGENTS.md"),
            "Verify with soaplabs:verify-app, follow soaplabs:kept.",
        )
        .unwrap();
        let noticed = std::sync::Mutex::default();

        let first = unresolved_notice(&config, "fix the bug", &noticed).await;
        let again = unresolved_notice(&config, "fix the bug", &noticed).await;

        let first = first.expect("a notice for the pruned skill");
        assert!(first.contains(
            "- `soaplabs:verify-app`, named in AGENTS.md in repo: plugin `soaplabs`, \
             https://github.com/acme/skills at skills/verify-app/SKILL.md"
        ));
        assert!(!first.contains("soaplabs:kept"));
        assert_eq!(again, None);
    }
}
