//! Default commands seeded on first access.

use crate::db::models::CommandRow;

/// The 6 default commands.
pub fn default_commands() -> Vec<CommandRow> {
    vec![
        CommandRow {
            name: "commit".to_string(),
            description: "Commit changes".to_string(),
            prompt: "commit the relevant changes".to_string(),
            conditions: None,
            icon: Some("git-commit".to_string()),
            variant: None,
            active: Some(true),
            will_not_resume: None,
            agent_mode: None,
        },
        CommandRow {
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
        CommandRow {
            name: "create-pr".to_string(),
            description: "Create pull request".to_string(),
            prompt: "create a pull request with the changes".to_string(),
            conditions: None,
            icon: Some("git-pull-request".to_string()),
            variant: None,
            active: Some(true),
            will_not_resume: None,
            agent_mode: None,
        },
        CommandRow {
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
        CommandRow {
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
        CommandRow {
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
