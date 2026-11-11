//! The person a Task's commits credit beside the agent, and how each harness is
//! told to add them.
//!
//! The backend sends [`GIT_CO_AUTHOR`] only when the sandbox commits as an
//! identity other than the person the Task credits, so its absence means
//! nobody to credit.

use std::collections::HashMap;

use moatless_types::credential_key::GIT_CO_AUTHOR;

/// The `Name <email>` a credential map names, if any.
pub(crate) fn co_author(credentials: &HashMap<String, String>) -> Option<&str> {
    credentials
        .get(GIT_CO_AUTHOR)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

/// Claude Code's `attribution.commit` crediting `co_author` beside Claude.
///
/// Setting the key replaces the CLI's own trailer, so this writes Claude's line
/// too. It names no model rather than the CLI's per-commit one: the file is
/// written once, and a Task can switch model mid-run.
pub(crate) fn claude_commit_attribution(co_author: &str) -> String {
    format!("Co-Authored-By: Claude <noreply@anthropic.com>\nCo-authored-by: {co_author}")
}

/// `system_prompt` with a note asking Codex to credit the co-author `env`
/// names, appended; `system_prompt` unchanged when it names none.
///
/// Codex has no attribution setting: its own trailer is a ChatGPT workspace
/// policy, so the note leaves any other trailer in place.
pub(crate) fn with_codex_commit_note(
    system_prompt: Option<&str>,
    env: Option<&HashMap<String, String>>,
) -> Option<String> {
    let Some(co_author) = env.and_then(co_author) else {
        return system_prompt.map(str::to_string);
    };
    let note = codex_commit_note(co_author);
    Some(match system_prompt.filter(|prompt| !prompt.is_empty()) {
        Some(prompt) => format!("{prompt}\n\n{note}"),
        None => note,
    })
}

fn codex_commit_note(co_author: &str) -> String {
    format!(
        "## Commit attribution\n\
         This task's work is for {co_author}. End every git commit message you \
         write with the trailer `Co-authored-by: {co_author}`, exactly once, in \
         the trailer block after one blank line. Keep any other trailers."
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{GIT_CO_AUTHOR, co_author, with_codex_commit_note};

    #[test]
    fn a_blank_co_author_credits_nobody() {
        for value in ["", "  "] {
            let credentials = HashMap::from([(GIT_CO_AUTHOR.to_string(), value.to_string())]);
            assert_eq!(co_author(&credentials), None);
        }
        assert_eq!(co_author(&HashMap::new()), None);
    }

    #[test]
    fn codex_is_told_to_credit_the_co_author_after_the_backend_prompt() {
        let env = HashMap::from([(
            GIT_CO_AUTHOR.to_string(),
            "Ada <ada@example.com>".to_string(),
        )]);
        let prompt = with_codex_commit_note(Some("backend prompt"), Some(&env)).unwrap();
        assert!(prompt.starts_with("backend prompt\n\n## Commit attribution"));
        assert!(prompt.contains("`Co-authored-by: Ada <ada@example.com>`"));
        assert!(
            with_codex_commit_note(None, Some(&env))
                .unwrap()
                .starts_with("## Commit attribution")
        );
    }

    #[test]
    fn without_a_co_author_codex_gets_the_backend_prompt_untouched() {
        assert_eq!(
            with_codex_commit_note(Some("backend prompt"), Some(&HashMap::new())).as_deref(),
            Some("backend prompt")
        );
        assert_eq!(with_codex_commit_note(None, None), None);
    }
}
