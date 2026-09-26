//! The recording sink every case here asserts against, and the thread, viewer
//! and command values it runs one against.

use super::*;

/// What the sink was asked to do. Every assertion about the command table is an
/// assertion about this list, because the table's whole job is deciding which
/// of these happens.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Call {
    SendTurn {
        task_id: String,
        message_uuid: String,
        model: Option<String>,
        agent_mode: Option<String>,
        effort: Option<String>,
        /// Each attachment as it reached the sink, in the order it was sent.
        attachments: Vec<Attached>,
        skills: Vec<String>,
    },
    CreateThread {
        task_id: String,
        workspace_id: String,
        instance: String,
        branch: Option<String>,
        has_first_message: bool,
        provision: bool,
        skills: Vec<String>,
    },
    Fork {
        new_task_id: String,
        source_task_id: String,
        at_turn: Option<i32>,
        message: Option<String>,
        same_sandbox: bool,
        model: Option<String>,
        agent_mode: Option<String>,
        effort: Option<String>,
    },
    Revert {
        task_id: String,
        keep_through: i32,
    },
    /// A command's `harness`, recorded before the call carrying it.
    Switch {
        task_id: String,
        harness: Harness,
    },
    UpdateMeta {
        task_id: String,
        title: Option<String>,
        model: Option<String>,
    },
    Interrupt {
        task_id: String,
    },
    Stop {
        task_id: String,
    },
    Archive {
        task_id: String,
    },
    Unarchive {
        task_id: String,
    },
    Follow {
        task_id: String,
    },
    Unfollow {
        task_id: String,
    },
    Settle {
        task_id: String,
    },
    Unsettle {
        task_id: String,
    },
    Snooze {
        task_id: String,
        snoozed_until: String,
    },
    Unsnooze {
        task_id: String,
    },
    SetVisibility {
        task_id: String,
        visibility: Visibility,
    },
    AnswerQuestion {
        task_id: String,
        tool_use_id: String,
        tool_name: String,
        payload: Option<Value>,
    },
    SetPullRequestLink {
        task_id: String,
        host: String,
        repository: String,
        number: i64,
        linked: bool,
    },
}

/// One attachment as the sink received it. An inline one is recorded by what
/// its bytes turned out to be — not by what the client declared — and an
/// uploaded one by the only thing it carries.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Attached {
    Inline(String, String, usize),
    Uploaded(String, AttachmentKind),
}

impl From<TurnAttachment> for Attached {
    fn from(attachment: TurnAttachment) -> Self {
        match attachment {
            TurnAttachment::Inline {
                filename,
                mime_type,
                bytes,
            } => Self::Inline(filename, mime_type, bytes.len()),
            TurnAttachment::Uploaded {
                attachment_id,
                kind,
            } => Self::Uploaded(attachment_id, kind),
        }
    }
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<Call>>);

#[async_trait]
impl ThreadCommandSink for RecordingSink {
    async fn send_turn(
        &self,
        _viewer: &ScopeContext,
        turn: ThreadTurn,
    ) -> Result<(), CommandRefusal> {
        if let Some(harness) = turn.harness {
            self.0.lock().unwrap().push(Call::Switch {
                task_id: turn.task_id.clone(),
                harness,
            });
        }
        self.0.lock().unwrap().push(Call::SendTurn {
            task_id: turn.task_id,
            message_uuid: turn.message_uuid,
            model: turn.model,
            agent_mode: turn.agent_mode.map(|m| m.0),
            effort: turn.effort,
            attachments: turn.attachments.into_iter().map(Attached::from).collect(),
            skills: turn.skills,
        });
        Ok(())
    }
    async fn create_thread(
        &self,
        _viewer: &ScopeContext,
        thread: NewThread,
    ) -> Result<(), CommandRefusal> {
        self.0.lock().unwrap().push(Call::CreateThread {
            task_id: thread.task_id,
            workspace_id: thread.workspace_id,
            instance: thread.instance,
            branch: thread.branch,
            has_first_message: thread.first_message.is_some(),
            provision: thread.provision,
            skills: thread
                .first_message
                .map(|message| message.skills)
                .unwrap_or_default(),
        });
        Ok(())
    }
    async fn fork(&self, _viewer: &ScopeContext, fork: ForkThread) -> Result<(), CommandRefusal> {
        if let Some(harness) = fork.harness {
            self.0.lock().unwrap().push(Call::Switch {
                task_id: fork.new_task_id.clone(),
                harness,
            });
        }
        self.0.lock().unwrap().push(Call::Fork {
            new_task_id: fork.new_task_id,
            source_task_id: fork.source_task_id,
            at_turn: fork.at_turn,
            message: fork.message,
            same_sandbox: fork.same_sandbox,
            model: fork.model,
            agent_mode: fork.agent_mode.map(|m| m.0),
            effort: fork.effort,
        });
        Ok(())
    }
    async fn revert(
        &self,
        _viewer: &ScopeContext,
        task_id: &str,
        keep_through: i32,
    ) -> Result<(), CommandRefusal> {
        self.0.lock().unwrap().push(Call::Revert {
            task_id: task_id.to_string(),
            keep_through,
        });
        Ok(())
    }
    async fn update_meta(
        &self,
        _viewer: &ScopeContext,
        meta: ThreadMeta,
    ) -> Result<(), CommandRefusal> {
        if let Some(harness) = meta.harness {
            self.0.lock().unwrap().push(Call::Switch {
                task_id: meta.task_id.clone(),
                harness,
            });
        }
        self.0.lock().unwrap().push(Call::UpdateMeta {
            task_id: meta.task_id,
            title: meta.title,
            model: meta.model,
        });
        Ok(())
    }
    async fn interrupt(&self, _viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.0.lock().unwrap().push(Call::Interrupt {
            task_id: task_id.to_string(),
        });
        Ok(())
    }
    async fn stop(&self, _viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.0.lock().unwrap().push(Call::Stop {
            task_id: task_id.to_string(),
        });
        Ok(())
    }
    async fn set_idle_timeout(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
        _minutes: i32,
    ) -> Result<(), CommandRefusal> {
        unreachable!("no `thread.*` command changes an idle timeout")
    }
    async fn archive(&self, _viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.0.lock().unwrap().push(Call::Archive {
            task_id: task_id.to_string(),
        });
        Ok(())
    }
    async fn unarchive(&self, _viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.0.lock().unwrap().push(Call::Unarchive {
            task_id: task_id.to_string(),
        });
        Ok(())
    }
    async fn follow(&self, _viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.0.lock().unwrap().push(Call::Follow {
            task_id: task_id.to_string(),
        });
        Ok(())
    }
    async fn unfollow(&self, _viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.0.lock().unwrap().push(Call::Unfollow {
            task_id: task_id.to_string(),
        });
        Ok(())
    }
    async fn settle(&self, _viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.0.lock().unwrap().push(Call::Settle {
            task_id: task_id.to_string(),
        });
        Ok(())
    }
    async fn unsettle(&self, _viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.0.lock().unwrap().push(Call::Unsettle {
            task_id: task_id.to_string(),
        });
        Ok(())
    }
    async fn snooze(
        &self,
        _viewer: &ScopeContext,
        task_id: &str,
        snoozed_until: DateTime<Utc>,
    ) -> Result<(), CommandRefusal> {
        self.0.lock().unwrap().push(Call::Snooze {
            task_id: task_id.to_string(),
            snoozed_until: snoozed_until.to_rfc3339(),
        });
        Ok(())
    }
    async fn unsnooze(&self, _viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.0.lock().unwrap().push(Call::Unsnooze {
            task_id: task_id.to_string(),
        });
        Ok(())
    }
    async fn set_visibility(
        &self,
        _viewer: &ScopeContext,
        task_id: &str,
        visibility: Visibility,
    ) -> Result<(), CommandRefusal> {
        self.0.lock().unwrap().push(Call::SetVisibility {
            task_id: task_id.to_string(),
            visibility,
        });
        Ok(())
    }
    async fn answer_question(
        &self,
        _viewer: &ScopeContext,
        answer: QuestionAnswer,
    ) -> Result<(), CommandRefusal> {
        self.0.lock().unwrap().push(Call::AnswerQuestion {
            task_id: answer.task_id,
            tool_use_id: answer.tool_use_id,
            tool_name: answer.tool_name,
            payload: answer.payload,
        });
        Ok(())
    }
    async fn link_pull_request(
        &self,
        _viewer: &ScopeContext,
        reference: ThreadPullRequestRef,
    ) -> Result<(), CommandRefusal> {
        self.0
            .lock()
            .unwrap()
            .push(pull_request_call(reference, true));
        Ok(())
    }
    async fn unlink_pull_request(
        &self,
        _viewer: &ScopeContext,
        reference: ThreadPullRequestRef,
    ) -> Result<(), CommandRefusal> {
        self.0
            .lock()
            .unwrap()
            .push(pull_request_call(reference, false));
        Ok(())
    }
}

fn pull_request_call(reference: ThreadPullRequestRef, linked: bool) -> Call {
    Call::SetPullRequestLink {
        task_id: reference.task_id,
        host: reference.host,
        repository: reference.repository,
        number: reference.number,
        linked,
    }
}

/// The two skills `ws-1` offers every test here.
struct StubSkills;

#[async_trait]
impl SkillCatalog for StubSkills {
    async fn list_for_workspace(
        &self,
        _viewer: &ScopeContext,
        _caller: Uuid,
        _workspace_id: &str,
    ) -> Vec<crate::ports::WorkspaceSkill> {
        ["git-workflow", "linear"]
            .into_iter()
            .map(|name| crate::ports::WorkspaceSkill {
                name: name.to_string(),
                description: format!("The {name} skill"),
                plugin_name: None,
            })
            .collect()
    }
}

struct StubAgents;

#[async_trait]
impl AgentCatalogSource for StubAgents {
    async fn list(&self, _viewer: &ScopeContext) -> Vec<crate::agent::schemas::AgentDescriptor> {
        catalog::catalog()
            .iter()
            .map(|entry| crate::agent::schemas::AgentDescriptor {
                id: entry.id.to_string(),
                label: entry.label.to_string(),
                installed: true,
                default_model: entry.default_model.to_string(),
                models: entry
                    .models
                    .iter()
                    .map(|m| crate::agent::schemas::AgentModel {
                        id: m.id.to_string(),
                        label: m.label.to_string(),
                    })
                    .collect(),
                missing_credential_hint: None,
            })
            .collect()
    }
}

pub(super) fn conversation(agent_profile: &str) -> Conversation {
    Conversation {
        task_id: "task-1".to_string(),
        workspace_id: Some("ws-1".to_string()),
        name: None,
        branch: None,
        agent_profile: AgentProfile(agent_profile.to_string()),
        created_at: "2026-07-01T09:00:00.000Z".to_string(),
        created_at_db: None,
        updated_at: None,
        closed_at: None,
        visibility: Visibility::Private,
        owner_name: None,
        turn_number: None,
        turn_status: None,
        turn_started_at: None,
        turn_ended_at: None,
        sandbox_desired_state_raw: None,
        sandbox_stop_reason_raw: None,
        sandbox_started_at: None,
        sandbox_updated_at: None,
        running_commands: 0,
        latest_user_message_at: None,
        latest_queued_message_at: None,
        assistant_model: None,
        requested_model: None,
        pending_question: false,
        pending_plan: false,
        settled_override: None,
        settled_at: None,
        snoozed_until: None,
        snoozed_at: None,
        pull_requests: None,
    }
}

pub(super) fn named(name: &str) -> Conversation {
    Conversation {
        name: Some(name.to_string()),
        ..conversation(AgentProfile::CLAUDE_CODE)
    }
}

/// A DAO that answers with one conversation, or with none when `found` is
/// `None` — "the viewer cannot be shown it", from here.
///
/// Control is granted; the test that withholds it overrides
/// `controls_conversation` itself.
pub(super) fn conversations(found: Option<Conversation>) -> MockConversationDao {
    let mut dao = MockConversationDao::new();
    dao.expect_readable_by_id()
        .returning(move |_, _| Ok(found.clone()));
    dao.expect_controls_conversation()
        .returning(|_, _| Ok(true));
    dao.expect_listing_head().returning(|| {
        Ok(UiRpcListingHead {
            identity: UiRpcEnvironmentIdentity::new("test"),
            position: 42,
        })
    });
    dao
}

/// The stored call an answer names — the unanswered `AskUserQuestion` a panel
/// is derived from, unless a test says otherwise.
pub(super) fn asked(tool_name: &str) -> InteractiveCall {
    InteractiveCall {
        tool_name: tool_name.to_string(),
        settled: false,
    }
}

pub(super) fn viewer() -> ScopeContext {
    ScopeContext {
        user_id: "user-1".to_string(),
        is_admin: false,
    }
}

/// Run one command against a Claude thread that exists.
pub(super) async fn run(command: Value) -> (Result<Value, CommandRefusal>, Vec<Call>) {
    run_against(command, Some(conversation(AgentProfile::CLAUDE_CODE))).await
}

/// Run one command against a thread whose stored call is `call`.
pub(super) async fn run_answering(
    command: Value,
    call: Option<InteractiveCall>,
) -> (Result<Value, CommandRefusal>, Vec<Call>) {
    let mut dao = conversations(Some(conversation(AgentProfile::CLAUDE_CODE)));
    dao.expect_interactive_call()
        .returning(move |_, _| Ok(call.clone()));
    dispatch_against(dao, command).await
}

pub(super) async fn run_against(
    command: Value,
    found: Option<Conversation>,
) -> (Result<Value, CommandRefusal>, Vec<Call>) {
    let mut dao = conversations(found);
    dao.expect_interactive_call()
        .returning(|_, _| Ok(Some(asked(mapping::QUESTION_TOOL))));
    dispatch_against(dao, command).await
}

pub(super) async fn dispatch_against(
    dao: MockConversationDao,
    command: Value,
) -> (Result<Value, CommandRefusal>, Vec<Call>) {
    dispatch_with_workspaces(dao, MockWorkspaceScriptCommands::new(), command).await
}

pub(super) async fn dispatch_with_workspaces(
    dao: MockConversationDao,
    workspaces: MockWorkspaceScriptCommands,
    command: Value,
) -> (Result<Value, CommandRefusal>, Vec<Call>) {
    let sink = RecordingSink::default();
    let agents = StubAgents;
    let skills = StubSkills;
    let ctx = CommandContext {
        conversations: &dao,
        agents: &agents,
        sink: &sink,
        workspace_script_commands: &workspaces,
        skills: &skills,
        caller: Uuid::nil(),
    };
    let result = dispatch(&ctx, &viewer(), &command).await;
    let calls = std::mem::take(&mut *sink.0.lock().unwrap());
    (result, calls)
}

pub(super) fn turn(model: Option<(&str, &str)>) -> Value {
    let mut command = json!({
        "type": "thread.turn.start",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "message": {
            "messageId": "msg-1",
            "role": "user",
            "text": "have another look",
            "attachments": []
        },
        "interactionMode": "default",
        "createdAt": "2026-07-01T09:00:00.000Z"
    });
    if let Some((instance, slug)) = model {
        command["modelSelection"] = json!({ "instanceId": instance, "model": slug });
    }
    command
}

/// The client's answer to whatever the thread is waiting on.
pub(super) fn answer(answers: Value) -> Value {
    json!({
        "type": "thread.user-input.respond",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "requestId": "toolu_1",
        "answers": answers,
        "createdAt": "2026-07-01T09:00:00.000Z"
    })
}
