//! Warn when a turn's text or a checkout's instructions name a plugin skill
//! this sandbox was not given.
//!
//! A plugin's activation records can pick a subset of its skills, and
//! `init::plugin_skills::keep_only` sets the rest aside where the Skill tool
//! does not look. A LOOP.md, a brief, an `AGENTS.md` or a repository skill that
//! names `factory:spec` then points at nothing, and the agent only finds out
//! when the Skill tool answers "Unknown skill". The notice names the set-aside
//! `SKILL.md` to read instead.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use moatless_workspace_types::plugin::REPO_SKILL_ROOTS;
use tracing::warn;

use crate::config::Config;
use crate::init::plugin_skills::PluginNamespace;

/// The instruction files read at each checkout's root, beside the prompt.
const INSTRUCTION_FILES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// A notice naming each `namespace:skill` that `prompt`, or with
/// `with_instructions` a checkout's instruction files, reference, whose
/// namespace a delivered plugin declares, and whose `SKILL.md` is not on disk;
/// `None` when there is none.
pub(crate) async fn unresolved_notice(
    config: &Config,
    prompt: &str,
    with_instructions: bool,
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
        if with_instructions {
            sources.extend(instruction_files(&repo_dirs));
        }
        unresolved(&namespaces, &sources)
    })
    .await
    .unwrap_or_else(|e| {
        warn!(error = %e, "Skill reference scan failed");
        Vec::new()
    });

    if missing.is_empty() {
        return None;
    }
    Some(notice(&missing))
}

/// One reference that resolves to no delivered skill.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Missing {
    reference: String,
    source: String,
    plugin: String,
    git_url: String,
    skill_path: String,
    /// The set-aside `SKILL.md`; `None` when the plugin has no such skill.
    unpicked: Option<PathBuf>,
}

/// Each checkout's root instruction files and repository skills as
/// `(label, text)`, a file reached twice through a symlink read once.
///
/// A repository skill is read rather than left to its own turn: a
/// Prerequisites line naming `infra:prod-change` is followed when the skill
/// loads, and no notice reaches that turn.
fn instruction_files(repo_dirs: &[String]) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for repo_dir in repo_dirs {
        let repo = Path::new(repo_dir);
        let repo_name = repo
            .file_name()
            .map_or_else(|| repo_dir.clone(), |n| n.to_string_lossy().into_owned());
        let mut paths: Vec<String> = INSTRUCTION_FILES.iter().map(|f| (*f).to_string()).collect();
        paths.extend(repo_skill_files(repo));
        for file in paths {
            let path = repo.join(&file);
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

/// Every `<root>/<skill>/SKILL.md` under `repo`'s [`REPO_SKILL_ROOTS`],
/// relative to `repo`, sorted within each root.
fn repo_skill_files(repo: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for root in REPO_SKILL_ROOTS {
        let Ok(entries) = std::fs::read_dir(repo.join(root)) else {
            continue;
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        files.extend(names.into_iter().map(|n| format!("{root}/{n}/SKILL.md")));
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
            let unpicked = ns.unpicked_dir.join(skill).join("SKILL.md");
            missing.push(Missing {
                reference,
                source: source.clone(),
                plugin: ns.plugin.clone(),
                git_url: ns.git_url.clone(),
                skill_path: format!("{}/{skill}/SKILL.md", ns.repo_skills_dir),
                unpicked: unpicked.is_file().then_some(unpicked),
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
        "Named but not delivered to this sandbox: the activation records for this Workspace and \
         run-as user leave these plugin skills out, so the Skill tool will not find them. Where a \
         path is given, read that `SKILL.md` and follow it as the Skill tool would have loaded \
         it. Tell the user which were missing; an admin turns one on with `moat plugins enable`.\n",
    );
    for m in missing {
        let _ = match &m.unpicked {
            Some(path) => writeln!(
                out,
                "- `{}`, named in {}: read {}",
                m.reference,
                m.source,
                path.display()
            ),
            None => writeln!(
                out,
                "- `{}`, named in {}: not in this sandbox's copy of plugin `{}`, {} at {}",
                m.reference, m.source, m.plugin, m.git_url, m.skill_path
            ),
        };
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn namespace(root: &Path, name: &str) -> PluginNamespace {
        PluginNamespace {
            name: name.to_string(),
            plugin: "soaplabs".to_string(),
            git_url: "https://github.com/acme/skills".to_string(),
            repo_skills_dir: format!("plugins/{name}/skills"),
            skills_dir: root.join(name),
            unpicked_dir: root.join(".unpicked").join(name),
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
                unpicked: None,
            }]
        );
    }

    #[test]
    fn a_set_aside_skill_is_reported_with_the_path_to_read() {
        let tmp = tempfile::tempdir().unwrap();
        with_skill(&tmp.path().join(".unpicked"), "factory", "build");
        let namespaces = [namespace(tmp.path(), "factory")];
        let sources = [("this message".to_string(), "Use factory:build.".to_string())];

        let missing = unresolved(&namespaces, &sources);
        let path = tmp.path().join(".unpicked/factory/build/SKILL.md");

        assert_eq!(missing[0].unpicked.as_deref(), Some(path.as_path()));
        assert!(notice(&missing).contains(&format!(
            "- `factory:build`, named in this message: read {}",
            path.display()
        )));
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
    async fn an_agents_md_reference_is_named_only_with_the_instructions() {
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
        let first = unresolved_notice(&config, "fix the bug", true).await;
        let again = unresolved_notice(&config, "fix the bug", false).await;

        let first = first.expect("a notice for the pruned skill");
        assert!(first.contains(
            "- `soaplabs:verify-app`, named in AGENTS.md in repo: not in this sandbox's copy of \
             plugin `soaplabs`, https://github.com/acme/skills at skills/verify-app/SKILL.md"
        ));
        assert!(!first.contains("soaplabs:kept"));
        assert_eq!(again, None);
    }

    #[test]
    fn a_repository_skill_is_read_once_through_its_symlinked_root() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("soap-app-gitops");
        let skill = repo.join(".agents/skills/release-train");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "Load infra:prod-change first.").unwrap();
        std::fs::create_dir_all(repo.join(".claude")).unwrap();
        std::os::unix::fs::symlink("../.agents/skills", repo.join(".claude/skills")).unwrap();

        let files = instruction_files(&[repo.display().to_string()]);

        assert_eq!(
            files,
            [(
                ".claude/skills/release-train/SKILL.md in soap-app-gitops".to_string(),
                "Load infra:prod-change first.".to_string(),
            )]
        );
    }
}
