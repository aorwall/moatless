//! `moat react` — emoji reactions on existing adapter messages.
//!
//! Looks up an internal message id in the backend, resolves its adapter and
//! external id, and calls the platform reaction API (Slack, GitHub PR, Linear,
//! Telegram). v1 is fire-and-forget — no reaction row persisted on our side.

use clap::Args;
use moatless_api_schemas::message::AddMessageReactionRequest;

use crate::client::MoatClient;
use crate::config::Config;
use crate::output;

#[derive(Args)]
pub struct ReactArgs {
    /// Internal message ID to react to (`task_messages.uuid`).
    #[arg(long)]
    pub message: String,
    /// Emoji shortcode (e.g. "thumbsup", "+1", "rocket"). Platform-native names accepted.
    #[arg(long, short = 'e')]
    pub emoji: String,
    /// Remove a previously added reaction instead of adding one.
    #[arg(long)]
    pub remove: bool,
}

pub async fn run(
    moat: &MoatClient,
    args: ReactArgs,
    _cfg: &Config,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if args.remove {
        moat.api()
            .remove_message_reaction(&args.message, &args.emoji)
            .await?;
        if json {
            output::print_json_typed(&serde_json::json!({
                "ok": true,
                "removed": true,
            }));
        } else {
            println!(
                "Removed reaction {} from message {}.",
                args.emoji, args.message
            );
        }
        return Ok(());
    }

    let req = AddMessageReactionRequest {
        emoji: args.emoji.clone(),
    };
    let resp = moat
        .api()
        .add_message_reaction(&args.message, &req)
        .await?;
    if json {
        output::print_json_typed(&resp);
    } else {
        println!(
            "Reacted with {} to message {}.",
            args.emoji, args.message
        );
    }
    Ok(())
}
