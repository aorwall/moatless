//! Bot user management — `moat bot create --app-key <key>` provisions a `users`
//! row tied to a configured GitHub App (`auth_method =
//! 'github_app_installation'` on the matching `provider_tokens` row).

use clap::Subcommand;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum BotAction {
    /// Create a bot user backed by a configured GitHub App.
    ///
    /// Resolves the App's `{slug}[bot]` identity, writing both the `users` and
    /// matching `provider_tokens` rows; triggers can set `run_as_user_id` to
    /// the returned id to attribute commits/PRs/comments to the bot.
    Create {
        /// Registry key of the GitHub App, e.g. `dev-bot`.
        #[arg(long)]
        app_key: String,
    },
}

pub async fn run(moat: &MoatClient, action: BotAction, json: bool) -> Result<(), ClientError> {
    match action {
        BotAction::Create { app_key } => {
            let body = serde_json::json!({
                "isBot": true,
                "githubAppKey": app_key,
            });
            let resp = moat.raw_post("/api/v1/users", &body).await?;
            output::emit(json, &resp, || {
                println!(
                    "Bot user created: id={} login={}",
                    resp["id"].as_str().unwrap_or(""),
                    resp["login"].as_str().unwrap_or(""),
                );
            });
        }
    }
    Ok(())
}
