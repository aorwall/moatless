//! Slack slash command payload types and parsing.

use serde::Deserialize;

/// Payload from a Slack slash command POST (`application/x-www-form-urlencoded`).
///
/// Slack sends this when a user invokes a registered slash command (e.g., `/moat commit fix bug`).
/// See: <https://api.slack.com/interactivity/slash-commands>
#[derive(Debug, Deserialize)]
pub struct SlackSlashCommandPayload {
    /// The slash command that was invoked (e.g., "/moat" or "/commit").
    pub command: String,
    /// Text after the command (e.g., "commit fix auth bug" for `/moat commit fix auth bug`).
    #[serde(default)]
    pub text: String,
    /// URL for sending delayed responses (valid for 30 minutes).
    pub response_url: String,
    /// Short-lived ID for opening modals.
    pub trigger_id: String,
    /// Slack user ID of the invoker.
    pub user_id: String,
    /// Slack channel ID where the command was typed.
    pub channel_id: String,
    /// Slack channel name (may be absent for DMs).
    #[serde(default)]
    pub channel_name: Option<String>,
    /// Slack workspace/team ID.
    pub team_id: String,
}

/// Parsed command extracted from a slash command payload.
pub struct ParsedSlashCommand {
    /// System command name (e.g., "commit", "create-pr").
    pub command_name: String,
    /// Remaining user text after the command name.
    pub user_text: String,
}

impl SlackSlashCommandPayload {
    /// Extract the system command name and user text from the payload.
    ///
    /// Strategy:
    /// 1. If the slash command itself (stripped of `/`) is NOT a gateway name,
    ///    treat it as a direct system command (all `text` is user context).
    /// 2. Otherwise (gateway like `/moat`), the first word of `text` is the
    ///    system command name, and the rest is user context.
    ///
    /// Returns `None` if no command name can be extracted (e.g., bare `/moat`
    /// with no text).
    pub fn parse_command(&self, gateway_commands: &[&str]) -> Option<ParsedSlashCommand> {
        let bare_command = self.command.trim_start_matches('/');

        // Direct system command (e.g., `/commit fix bug`)
        if !gateway_commands.contains(&bare_command) {
            return Some(ParsedSlashCommand {
                command_name: bare_command.to_string(),
                user_text: self.text.trim().to_string(),
            });
        }

        // Gateway command (e.g., `/moat commit fix bug`)
        let trimmed = self.text.trim();
        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let cmd = parts.next()?.trim();
        if cmd.is_empty() {
            return None;
        }
        let rest = parts.next().unwrap_or("").trim();

        Some(ParsedSlashCommand {
            command_name: cmd.to_string(),
            user_text: rest.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(command: &str, text: &str) -> SlackSlashCommandPayload {
        SlackSlashCommandPayload {
            command: command.to_string(),
            text: text.to_string(),
            response_url: String::new(),
            trigger_id: String::new(),
            user_id: String::new(),
            channel_id: String::new(),
            channel_name: None,
            team_id: String::new(),
        }
    }

    #[test]
    fn gateway_with_command_and_text() {
        let p = payload("/moat", "commit fix auth bug");
        let parsed = p.parse_command(&["moat"]).unwrap();
        assert_eq!(parsed.command_name, "commit");
        assert_eq!(parsed.user_text, "fix auth bug");
    }

    #[test]
    fn gateway_with_command_only() {
        let p = payload("/moat", "commit");
        let parsed = p.parse_command(&["moat"]).unwrap();
        assert_eq!(parsed.command_name, "commit");
        assert_eq!(parsed.user_text, "");
    }

    #[test]
    fn gateway_with_no_text() {
        let p = payload("/moat", "");
        assert!(p.parse_command(&["moat"]).is_none());
    }

    #[test]
    fn direct_command_with_text() {
        let p = payload("/commit", "fix auth bug");
        let parsed = p.parse_command(&["moat"]).unwrap();
        assert_eq!(parsed.command_name, "commit");
        assert_eq!(parsed.user_text, "fix auth bug");
    }

    #[test]
    fn direct_command_no_text() {
        let p = payload("/commit", "");
        let parsed = p.parse_command(&["moat"]).unwrap();
        assert_eq!(parsed.command_name, "commit");
        assert_eq!(parsed.user_text, "");
    }
}
