//! Unified `moat send` command — drives a task, DMs a user, or pass-through-sends
//! through any adapter destination. Replies to specific inbound messages live
//! under `moat messages reply --message <id>`.

use clap::{ArgGroup, Args};
use moatless_api_schemas::adapters::AdapterSendRequest;
use moatless_api_schemas::enums::AdapterKind;
use moatless_api_schemas::message::SendMessageRequest;
use moatless_api_schemas::shared::AgentMode;
use moatless_api_schemas::user::SendDmRequest;

use crate::client::MoatClient;
use crate::commands::misc::{read_text_or_stdin, upload_files};
use crate::config::Config;
use crate::output;

#[derive(Args)]
#[command(group(
    ArgGroup::new("send_target")
        .required(true)
        .args(["task", "user", "to"])
))]
pub struct SendArgs {
    /// Drive the agent on a task by ID
    #[arg(long, group = "send_target")]
    pub task: Option<String>,
    /// Agent mode for `--task`: "edit" or "plan"
    #[arg(long, requires = "task")]
    pub mode: Option<String>,

    /// DM a moatless user by login
    #[arg(long, group = "send_target")]
    pub user: Option<String>,

    /// Generic adapter pass-through: adapter-specific destination string
    /// (e.g. `channel:C123`, `dm:D456`, `thread:C123/172...`, `issue:LIN-12`).
    /// Pair with `--via`.
    #[arg(long, group = "send_target", requires = "via")]
    pub to: Option<String>,

    /// Adapter selector. For `--user`: provider ID type to look up
    /// (slack | telegram). For `--to`: adapter kind to dispatch through.
    #[arg(long)]
    pub via: Option<String>,

    /// Explicit connection ID for `--to`. Auto-resolves if there's only one.
    #[arg(long, requires = "to")]
    pub connection: Option<String>,

    /// Message text. Reads from stdin if omitted and stdin is piped.
    #[arg(long)]
    pub text: Option<String>,

    /// File(s) to attach. Repeatable.
    #[arg(long = "file")]
    pub files: Vec<String>,

    /// Task ID to bind the send to. Defaults to $TASK_ID for
    /// `--user`/`--to`.
    #[arg(long)]
    pub task_id: Option<String>,
}

pub async fn run(
    moat: &MoatClient,
    args: SendArgs,
    cfg: &Config,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let text = read_text_or_stdin(args.text)?;
    let file_ids = upload_files(moat, &args.files).await?;
    let task_id_ctx: Option<String> = args
        .task_id
        .clone()
        .or_else(|| cfg.task_id().map(String::from));

    if let Some(id) = args.task {
        if !file_ids.is_empty() {
            return Err("--file is not supported with --task".into());
        }
        let req = SendMessageRequest {
            message: text,
            agent_mode: args.mode.map(AgentMode),
            command_name: None,
            contexts: Vec::new(),
            file_ids: Vec::new(),
            model: None,
            resume: None,
            tool_name: None,
            tool_response: None,
            tool_use_id: None,
            source_task_id: cfg.task_id().map(std::string::ToString::to_string),
            start_fresh: None,
        };
        let resp = moat.api().send_message(&id, &req).await?;
        if json {
            output::print_json_typed(&resp);
        } else {
            println!("Message sent to task {id}.");
        }
        return Ok(());
    }

    if let Some(login) = args.user {
        if !file_ids.is_empty() {
            return Err("--file is not supported with --user".into());
        }
        let provider = args.via.clone().unwrap_or_else(|| "slack".to_string());
        let req = SendDmRequest {
            provider,
            text,
            task_id: task_id_ctx.clone(),
        };
        let resp = moat.api().send_user_dm(&login, &req).await?;
        if json {
            output::print_json_typed(&resp);
        } else {
            println!(
                "DM sent to {login}. Conversation: {}, ts: {}{}",
                resp.conversation_id,
                resp.message_ts,
                if resp.binding_id.is_empty() {
                    String::new()
                } else {
                    format!(", binding: {}", resp.binding_id)
                }
            );
        }
        return Ok(());
    }

    if let Some(destination) = args.to {
        let via = args
            .via
            .as_deref()
            .ok_or("--via is required with --to")?;
        let adapter = AdapterKind::parse(via)
            .ok_or_else(|| format!("Unknown adapter: {via}"))?;
        let req = AdapterSendRequest {
            adapter,
            connection_id: args.connection,
            destination,
            text,
            file_ids,
            task_id: task_id_ctx.clone(),
        };
        let resp = moat.api().adapter_send(&req).await?;
        if json {
            output::print_json_typed(&resp);
        } else {
            print!("Sent via {} ({}).", via, resp.subject_kind);
            if let Some(binding_id) = &resp.binding_id {
                print!(" Binding: {binding_id}.");
            }
            println!(" External id: {}", resp.external_id);
        }
        return Ok(());
    }

    Err("No target specified. Pass one of --task, --user, --to".into())
}
