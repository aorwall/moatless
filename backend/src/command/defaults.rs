//! Default commands seeded on first access.

use super::models::Command;

/// The 6 default commands.
pub fn default_commands() -> Vec<Command> {
    vec![
        Command {
            name: "commit".to_string(),
            description: "Commit changes".to_string(),
            prompt: "Review the changes with git diff, then commit them to a new branch with a clear, \
conventional commit message (e.g. feat:, fix:, refactor:). Push the branch to the remote."
                .to_string(),
            conditions: None,
            icon: Some("git-commit".to_string()),
            variant: None,
            active: Some(true),
            will_not_resume: None,
            agent_mode: None,
        },
        Command {
            name: "continue".to_string(),
            description: "Continue task".to_string(),
            prompt: "continue with the task".to_string(),
            conditions: None,
            icon: Some("play".to_string()),
            variant: None,
            active: Some(true),
            will_not_resume: None,
            agent_mode: None,
        },
        Command {
            name: "create-pr".to_string(),
            description: "Create pull request".to_string(),
            prompt: "Create a pull request for the current branch. Write a short title (under 70 chars) \
and a description body with a summary (bullet points) and a test plan. Push the branch if needed, \
then use `gh pr create`."
                .to_string(),
            conditions: None,
            icon: Some("git-pull-request".to_string()),
            variant: None,
            active: Some(true),
            will_not_resume: None,
            agent_mode: None,
        },
        Command {
            name: "merge-to-main".to_string(),
            description: "Merge to main".to_string(),
            prompt: "merge the changes to the main branch".to_string(),
            conditions: None,
            icon: Some("git-merge".to_string()),
            variant: None,
            active: Some(true),
            will_not_resume: None,
            agent_mode: None,
        },
        Command {
            name: "sync-with-main".to_string(),
            description: "Sync with main".to_string(),
            prompt: "sync the branch with the latest changes from main".to_string(),
            conditions: None,
            icon: Some("refresh-cw".to_string()),
            variant: None,
            active: Some(true),
            will_not_resume: None,
            agent_mode: None,
        },
        Command {
            name: "compact".to_string(),
            description: "Compact context".to_string(),
            prompt: "Summarize the conversation and compact the context".to_string(),
            conditions: None,
            icon: Some("minimize".to_string()),
            variant: None,
            active: Some(true),
            will_not_resume: Some(true),
            agent_mode: None,
        },
    ]
}
