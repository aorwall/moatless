//! `moat feedback` — sandbox-only command for the agent to record
//! qualitative feedback about the current task.

use moatless_api_schemas::feedback::{CreateFeedbackRequest, FeedbackSurface};

use crate::client::{ClientError, MoatClient};
use crate::output;

pub struct FeedbackArgs {
    pub task_id: String,
    pub comment: String,
    pub message_uuid: Option<String>,
    pub tool_use_id: Option<String>,
}

pub async fn run(
    client: &MoatClient,
    args: FeedbackArgs,
    json: bool,
) -> Result<(), ClientError> {
    let surface = if args.tool_use_id.is_some() {
        FeedbackSurface::ToolCall
    } else if args.message_uuid.is_some() {
        FeedbackSurface::Message
    } else {
        FeedbackSurface::Task
    };

    let req = CreateFeedbackRequest {
        task_id: args.task_id,
        message_uuid: args.message_uuid,
        tool_use_id: args.tool_use_id,
        surface,
        comment: args.comment,
        metadata: None,
    };

    let resp = client.api().submit_feedback(&req).await?;

    if json {
        output::print_json_typed(&resp);
    } else {
        println!("Recorded feedback {}", resp.feedback_id);
    }
    Ok(())
}
