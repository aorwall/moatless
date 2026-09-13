//! Moatless skills, as T3's provider skills.
//!
//! T3 discovers a provider's skills per working directory and offers them under
//! `/` and `$` in the composer. Moatless discovers them per Workspace, so this
//! module is the two halves of that translation: a `cwd` back to the Workspace
//! it names ([`workspace_of`]), and the picked `$name` back out of the turn the
//! composer sent ([`activated`]).

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::dao::ConversationDao;
use crate::nominal_path;
use crate::ports::WorkspaceSkill;
use moatless_types::access_control::ScopeContext;

/// The Workspace a T3 `cwd` names, whichever of the two forms it takes.
///
/// A draft thread's composer addresses its project root; a started one
/// addresses the thread's worktree. `None` for a path this module never issued
/// or a Task this viewer cannot see.
pub(crate) async fn workspace_of(
    conversations: &dyn ConversationDao,
    viewer: &ScopeContext,
    cwd: &str,
) -> Option<String> {
    if let Some(workspace_id) = nominal_path::workspace_id_from_project_root(cwd) {
        return Some(workspace_id.to_string());
    }
    let task_id = nominal_path::task_id_from_cwd(cwd)?;
    match conversations.readable_by_id(viewer, task_id).await {
        Ok(conversation) => conversation?.workspace_id,
        Err(error) => {
            tracing::warn!(%error, task_id, "T3 skill listing could not resolve its thread");
            None
        }
    }
}

/// One `cwd`'s skills, in the shape `ServerProviderWorkspaceSnapshot` decodes
/// from.
///
/// `slashCommands` stays empty: Moatless has no per-repository command surface,
/// and a skill is offered under `/` in its own right.
pub(crate) fn snapshot(cwd: &str, checked_at: &str, skills: &[WorkspaceSkill]) -> Value {
    json!({
        "cwd": cwd,
        "checkedAt": checked_at,
        "slashCommands": [],
        "skills": skills.iter().map(published).collect::<Vec<_>>(),
    })
}

/// One skill in the shape `ServerProviderSkill` decodes from.
///
/// `path` is nominal and the only field T3 classifies a skill by: a plugin path
/// earns the "App" badge, `scope` decides the rest. Moatless resolves the body
/// by name inside the Sandbox, so no reader needs the real file.
fn published(skill: &WorkspaceSkill) -> Value {
    let mut published = match &skill.plugin_name {
        Some(plugin) => json!({
            "name": skill.name,
            "path": format!("/.agents/plugins/{plugin}/skills/{}/SKILL.md", skill.name),
            "scope": plugin,
            "enabled": true,
        }),
        None => json!({
            "name": skill.name,
            "path": format!(".claude/skills/{}/SKILL.md", skill.name),
            "scope": "repo",
            "enabled": true,
        }),
    };
    // The contract's `description` refuses an empty string and has no null, so
    // a skill whose front matter declares none omits the key.
    let description = skill.description.trim();
    if !description.is_empty() {
        published["description"] = json!(description);
    }
    published
}

/// The skills a turn activates, in the order they appear in its text.
///
/// The composer leaves a `$name` mention in the prompt text unchanged; only a
/// mention naming a skill in `known` counts, keeping a `$HOME` in prose from
/// activating anything. Duplicates collapse to the first.
pub(crate) fn activated(text: &str, known: &HashSet<&str>) -> Vec<String> {
    let mut activated: Vec<String> = Vec::new();
    for mention in mentions(text) {
        if known.contains(mention) && !activated.iter().any(|name| name == mention) {
            activated.push(mention.to_string());
        }
    }
    activated
}

/// Whether `text` carries any `$name` token at all.
///
/// The cheap half of [`activated`], so a turn that picked no skill never reads
/// the Workspace's skill listing.
pub(crate) fn mentions_a_skill(text: &str) -> bool {
    mentions(text).next().is_some()
}

/// Every `$name` token in `text`, in order.
///
/// Matches the composer/timeline chip token shape exactly, so what renders as a
/// chip and what activates are one set: whitespace-bounded `$`, name starting
/// with a letter.
fn mentions(text: &str) -> impl Iterator<Item = &str> {
    text.split_whitespace()
        .filter_map(|token| token.strip_prefix('$'))
        .filter(|name| {
            name.starts_with(|c: char| c.is_ascii_alphabetic())
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str, description: &str, plugin: Option<&str>) -> WorkspaceSkill {
        WorkspaceSkill {
            name: name.to_string(),
            description: description.to_string(),
            plugin_name: plugin.map(str::to_string),
        }
    }

    #[test]
    fn a_repository_skill_is_published_under_the_repo_badge() {
        let published = snapshot(
            "/moatless/workspaces/ws-1",
            "2026-01-01T00:00:00.000Z",
            &[skill("git-workflow", "Commit and PR flow", None)],
        );

        assert_eq!(published["cwd"], "/moatless/workspaces/ws-1");
        assert_eq!(published["slashCommands"], json!([]));
        let skill = &published["skills"][0];
        assert_eq!(skill["name"], "git-workflow");
        assert_eq!(skill["description"], "Commit and PR flow");
        assert_eq!(skill["scope"], "repo");
        assert_eq!(skill["enabled"], true);
        assert_eq!(skill["path"], ".claude/skills/git-workflow/SKILL.md");
    }

    #[test]
    fn a_plugin_skill_is_published_under_a_path_t3_reads_as_a_plugin() {
        // `resolveProviderSkillSourceKind` classifies by path before scope, and
        // `/.agents/plugins/` is the segment that earns the "App" badge.
        let published = snapshot(
            "/moatless/tasks/task-1",
            "2026-01-01T00:00:00.000Z",
            &[skill("linear", "Issue tracking", Some("general"))],
        );

        let skill = &published["skills"][0];
        assert_eq!(skill["scope"], "general");
        assert_eq!(
            skill["path"],
            "/.agents/plugins/general/skills/linear/SKILL.md"
        );
    }

    #[test]
    fn a_skill_with_no_description_omits_the_key() {
        // The contract declares `description` optional and refuses an empty
        // string, so publishing `""` fails the whole config decode.
        let published = snapshot(
            "/moatless/workspaces/ws-1",
            "t",
            &[skill("bare", "  ", None)],
        );

        assert!(published["skills"][0].get("description").is_none());
    }

    #[test]
    fn only_a_mention_naming_a_known_skill_activates() {
        let known = HashSet::from(["git-workflow", "linear"]);

        assert_eq!(
            activated("use $git-workflow then $linear", &known),
            vec!["git-workflow".to_string(), "linear".to_string()]
        );
        assert_eq!(
            activated("echo $HOME and $PATH", &known),
            Vec::<String>::new()
        );
        assert_eq!(
            activated("$git-workflow twice $git-workflow", &known),
            vec!["git-workflow".to_string()]
        );
    }

    #[test]
    fn a_mention_inside_a_word_is_not_one() {
        // `$` only opens a token at a whitespace boundary, which is what the
        // composer's own chip pattern recognizes.
        let known = HashSet::from(["linear"]);

        assert_eq!(activated("cost$linear", &known), Vec::<String>::new());
        assert_eq!(
            activated("cost\n$linear\n", &known),
            vec!["linear".to_string()]
        );
    }
}
