//! Native `SdkMessage` → canonical `ProcessEvent` normalization.
//!
//! One `SourceEvent` per native content block, in native order, with repeated
//! usage forwarded verbatim (never coalesced). This is the only place that maps
//! Claude-native payloads; the shared harness/driver never sees an `SdkMessage`.

use claude_agent_sdk::types::content::ContentBlock;
use claude_agent_sdk::types::messages::{AssistantMessage, ToolProgressMessage, UserMessage};
use claude_agent_sdk::SdkMessage;

use super::super::process::ProcessEvent;
use super::super::signal::{Activity, SessionId};
use super::super::source::{NativeMessageId, Role, SourceEvent, SourceId};
use super::classify::classify_result;
use super::ClaudeProcess;

/// The per-message metadata shared by every content block of one native
/// `SdkMessage`, so a whole assistant/user envelope's blocks are normalized with
/// one grouping id, parent, usage, and model.
struct BlockMeta {
    session: SessionId,
    message_id: Option<String>,
    parent_tool_use_id: Option<String>,
    role: Role,
    usage: Option<serde_json::Value>,
    model: Option<String>,
}

impl ClaudeProcess {
    /// Build the canonical source event for one native content block, assigning a
    /// collision-safe id and preserving native grouping/usage/model.
    fn make_source(&mut self, meta: &BlockMeta, block: &ContentBlock) -> SourceEvent {
        let seq = self.block_seq;
        self.block_seq += 1;
        let native = meta
            .message_id
            .clone()
            .unwrap_or_else(|| format!("{}-anon-{seq}", self.nonce));
        SourceEvent {
            source_id: SourceId(format!("{native}#{seq}")),
            message_id: NativeMessageId(native),
            session: meta.session.clone(),
            parent_tool_use_id: meta.parent_tool_use_id.clone(),
            role: meta.role,
            block: serde_json::to_value(block).unwrap_or(serde_json::Value::Null),
            usage: meta.usage.clone(),
            model: meta.model.clone(),
            diagnostics: None,
        }
    }

    /// The Session as currently known, falling back to a native id when the
    /// process reports a block before its `System` init (defensive; the driver
    /// buffers early blocks regardless).
    fn session_or(&self, native: &str) -> SessionId {
        self.session
            .clone()
            .unwrap_or_else(|| SessionId(native.to_string()))
    }

    /// Turn one `SdkMessage` into zero or more `ProcessEvent`s, appended to the
    /// pending queue in native order.
    pub(super) fn absorb(&mut self, msg: SdkMessage) {
        match msg {
            SdkMessage::System(init) => {
                let session = SessionId(init.session_id.clone());
                self.session = Some(session.clone());
                if !self.session_ready {
                    self.session_ready = true;
                    self.pending
                        .push_back(ProcessEvent::SessionReady { session });
                }
                // The one-shot CLI accepted the prompt at spawn; the init proves
                // the process is up, so acceptance is known now.
                if !self.input_accepted {
                    self.input_accepted = true;
                    if let Some(input) = self.input_id.clone() {
                        self.pending
                            .push_back(ProcessEvent::InputAccepted { input });
                    }
                }
            }
            SdkMessage::Assistant(assistant) => self.absorb_assistant(assistant),
            SdkMessage::User(user) => self.absorb_user(user),
            SdkMessage::ToolProgress(progress) => self.absorb_progress(progress),
            SdkMessage::Result(result) => {
                self.pending.push_back(ProcessEvent::Terminated {
                    termination: classify_result(&result),
                });
            }
            // Partial streaming frames, replays, and system subtypes carry no
            // committed content block for this path.
            _ => {}
        }
    }

    fn absorb_assistant(&mut self, assistant: AssistantMessage) {
        let meta = BlockMeta {
            session: self.session_or(&assistant.session_id),
            message_id: assistant.message.id.clone(),
            parent_tool_use_id: assistant.parent_tool_use_id.clone(),
            role: Role::Assistant,
            usage: assistant
                .message
                .usage
                .as_ref()
                .and_then(|u| serde_json::to_value(u).ok()),
            model: assistant.message.model.clone(),
        };
        for block in &assistant.message.content {
            let event = self.make_source(&meta, block);
            self.pending.push_back(ProcessEvent::Block(event));
        }
    }

    fn absorb_user(&mut self, user: UserMessage) {
        let meta = BlockMeta {
            session: self.session_or(&user.session_id),
            message_id: user.uuid.clone(),
            parent_tool_use_id: user.parent_tool_use_id.clone(),
            role: Role::User,
            usage: None,
            model: None,
        };
        for block in &user.message.content {
            let event = self.make_source(&meta, block);
            self.pending.push_back(ProcessEvent::Block(event));
        }
    }

    fn absorb_progress(&mut self, progress: ToolProgressMessage) {
        self.pending.push_back(ProcessEvent::Activity(Activity {
            session: self.session_or(&progress.session_id),
            detail: progress.tool_name,
        }));
    }
}
