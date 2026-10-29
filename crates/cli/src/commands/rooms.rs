//! `moat rooms` — give a task its Task Room: a conversation space, such as a
//! Slack code channel, where every message reaches the task and its replies
//! land.

use clap::Subcommand;
use moatless_api_schemas::adapters::OpenTaskRoomRequest;
use moatless_api_schemas::enums::AdapterKind;

use crate::client::MoatClient;
use crate::config::Config;
use crate::output;

#[derive(Subcommand)]
pub enum RoomsAction {
    /// Open the task's room. It opens in the background and is bound to the
    /// task; a task has one room, so a second request does nothing
    Open {
        /// The adapter that hosts the room
        #[arg(long, default_value = "slack")]
        via: String,
        /// Where to announce the room, such as `channel:C123`. Omit to open it
        /// from the thread that started the task
        #[arg(long = "in", value_name = "DESTINATION")]
        anchor: Option<String>,
        /// Who to add: `requester`, `run_as`, or a Moatless login. Repeatable
        #[arg(long)]
        invite: Vec<String>,
        /// Make the room private
        #[arg(long, conflicts_with = "public")]
        private: bool,
        /// Make the room public
        #[arg(long)]
        public: bool,
        /// The room's name. Defaults to the task's name
        #[arg(long)]
        title: Option<String>,
        /// The `--via` connection to open it on. Auto-resolves if there's only
        /// one
        #[arg(long)]
        connection: Option<String>,
        /// The task to open it for. Defaults to `$TASK_ID`
        #[arg(long)]
        task: Option<String>,
    },
}

pub async fn run(
    moat: &MoatClient,
    action: RoomsAction,
    cfg: &Config,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        RoomsAction::Open {
            via,
            anchor,
            invite,
            private,
            public,
            title,
            connection,
            task,
        } => {
            let task_id = task
                .or_else(|| cfg.task_id().map(String::from))
                .ok_or("Pass --task <id>; no $TASK_ID is set")?;
            let via = AdapterKind::parse(&via).ok_or_else(|| format!("Unknown adapter: {via}"))?;
            let req = OpenTaskRoomRequest {
                via,
                connection_id: connection,
                anchor,
                invite,
                private: (private || public).then_some(private),
                title,
            };
            let resp = moat.api().open_task_room(&task_id, &req).await?;
            output::emit(json, &resp, || {
                if resp.requested {
                    println!(
                        "Room requested for task {task_id}; it opens shortly and is bound to the task."
                    );
                } else {
                    println!("Task {task_id} already has a room requested.");
                }
            });
            Ok(())
        }
    }
}
