//! The tasks band's construction root.
//!
//! [`task_graph`] owns every store this crate keeps private, and hands out only
//! independent capabilities over them. [`TaskGraph::assemble`] consumes the
//! graph and returns the workflow services, so a host never holds a store and
//! never re-wires one after the fact.

use std::sync::Arc;

use moatless_core::config::Config;
use moatless_core::db::DbPool;
use moatless_core::events::EventPublisher;
use moatless_core::file_storage::Files;
use moatless_identity::credentials::CredentialService;
use moatless_ports::identity::UserDisplay;
use moatless_ports::sandbox::{
    CredentialBroadcast, PreflightPort, SandboxJoiner, SandboxLifecycle, SandboxStarter,
};
use moatless_workspace_ports::{RepositoryReader, WorkspaceResolver};

use crate::artifact::dao::{ArtifactDao, PgArtifactDao};
use crate::artifact::operations::Artifacts;
use crate::credential_sync::CredentialSync;
use crate::event_ingress::EventAdmission;
use crate::feedback::dao::{FeedbackDao, PgFeedbackDao};
use crate::feedback::operations::FeedbackNotes;
use crate::message::dao::{MessageDao, PgMessageDao};
use crate::message::operations::Messages;
use crate::message::send::{MessageSendService, SelfEchoGuard};
use crate::review_guide::dao::{PgReviewGuideDao, ReviewGuideDao};
use crate::review_guide::operations::ReviewGuides;
use crate::tag::dao::{PgTagDao, TagDao};
use crate::tag::operations::Tags;
use crate::task::bindings::TaskBindings;
use crate::task::closer::TaskCloser;
use crate::task::creation::TaskCreationService;
use crate::task::dao::PgTaskDao;
use crate::task::fork::{TaskForkService, TaskForks};
use crate::task::fork_dao::{PgTaskForkDao, TaskForkDao};
use crate::task::operations::TaskCommands;
use crate::task::presenter::TaskPresenter;
use crate::task::relations::TaskRelations;
use crate::task::relations_dao::{PgTaskRelationDao, TaskRelationDao};
use crate::task::repositories::TaskRepositories;
use crate::task::repositories_dao::{PgTaskRepositoryDao, TaskRepositoryDao};
use crate::task::rewind::TaskRewindService;
use crate::task::switch::TaskSwitchService;
use crate::task_event::dao::{PgTaskEventDao, TaskEventDao};
use crate::task_event::operations::TaskEvents;
use crate::task_turn::dao::{PgTaskTurnDao, TaskTurnDao};
use crate::task_turn::operations::TaskTurns;
use crate::trajectory::dao::PgTrajectoryDao;
use crate::trajectory::operations::Trajectories;

/// Every store the tasks band owns, built once over one pool.
///
/// Cloning shares every store (each is an `Arc`), so a host that keeps the
/// graph alongside capabilities it already took assembles over the same
/// instances.
#[derive(Clone)]
pub struct TaskGraph {
    db: DbPool,
    tasks: Arc<PgTaskDao>,
    relations: Arc<dyn TaskRelationDao>,
    repositories: Arc<dyn TaskRepositoryDao>,
    forks: Arc<dyn TaskForkDao>,
    messages: Arc<dyn MessageDao>,
    turns: Arc<dyn TaskTurnDao>,
    events: Arc<dyn TaskEventDao>,
    tags: Arc<dyn TagDao>,
    feedback: Arc<dyn FeedbackDao>,
    review_guides: Arc<dyn ReviewGuideDao>,
    artifacts: Arc<dyn ArtifactDao>,
}

/// Build the tasks band's stores over `db`.
pub fn task_graph(db: DbPool) -> TaskGraph {
    TaskGraph {
        tasks: Arc::new(PgTaskDao::new(db.clone())),
        relations: Arc::new(PgTaskRelationDao(db.clone())),
        repositories: Arc::new(PgTaskRepositoryDao(db.clone())),
        forks: Arc::new(PgTaskForkDao(db.clone())),
        messages: Arc::new(PgMessageDao(db.clone())),
        turns: Arc::new(PgTaskTurnDao(db.clone())),
        events: Arc::new(PgTaskEventDao(db.clone())),
        tags: Arc::new(PgTagDao(db.clone())),
        feedback: Arc::new(PgFeedbackDao(db.clone())),
        review_guides: Arc::new(PgReviewGuideDao(db.clone())),
        artifacts: Arc::new(PgArtifactDao(db.clone())),
        db,
    }
}

impl TaskGraph {
    /// The `tasks` table, as a host reads and writes it.
    pub fn tasks(&self) -> Arc<TaskCommands> {
        Arc::new(TaskCommands::new(self.tasks.clone()))
    }

    /// The `tasks` table behind the published `TaskReader` seam.
    pub fn task_reader(&self) -> Arc<crate::ports::TaskProjection> {
        Arc::new(crate::ports::TaskProjection::new(self.tasks.clone()))
    }

    /// The session-gate port the auth `ForwardAuth` gate reads, so identity
    /// holds no `task` import.
    pub fn task_access(&self) -> Arc<dyn moatless_core::http_auth::TaskAccess> {
        self.tasks.clone()
    }

    /// The `task_relations` table, as a host reads and writes it. Also the
    /// published `RelationRecorder` seam.
    pub fn relation_recorder(&self) -> Arc<TaskRelations> {
        Arc::new(TaskRelations::new(self.relations.clone()))
    }

    /// The `task_repositories` table, as a host reads and writes it.
    pub fn task_repositories(&self) -> Arc<TaskRepositories> {
        Arc::new(TaskRepositories::new(self.repositories.clone()))
    }

    /// The `task_forks` table, as the delivery path reads it.
    pub fn fork_origins(&self) -> Arc<TaskForks> {
        Arc::new(TaskForks::new(self.forks.clone()))
    }

    /// The `trajectory_sessions` and `trajectory_events` tables.
    pub fn trajectory(&self) -> Arc<Trajectories> {
        Arc::new(Trajectories::new(PgTrajectoryDao(self.db.clone())))
    }

    /// A Task's conversation, as a host reads and writes it.
    pub fn messages(&self) -> Arc<Messages> {
        Arc::new(Messages::new(
            self.messages.clone(),
            crate::inbox::InboxService::new(self.db.clone()),
        ))
    }

    /// A Task's conversation behind the published `TranscriptAppend` seam.
    pub fn transcript(&self) -> Arc<crate::ports::Transcript> {
        Arc::new(crate::ports::Transcript::new(self.messages.clone()))
    }

    /// The `task_turns` table, as the ingress, the reaper and the event
    /// handlers read and write it.
    pub fn task_turns(&self) -> Arc<TaskTurns> {
        Arc::new(TaskTurns::new(self.turns.clone()))
    }

    /// The `task_turns` table behind the published `TurnCounter` and
    /// `TurnStateReader` seams.
    pub fn turn_ledger(&self) -> Arc<crate::ports::TurnLedger> {
        Arc::new(crate::ports::TurnLedger::new(self.turns.clone()))
    }

    /// The `task_events` table, as a host reads and writes it.
    pub fn task_events(&self) -> Arc<TaskEvents> {
        Arc::new(TaskEvents::new(self.events.clone()))
    }

    /// Durable admission for sandbox-published events, over the turn and event
    /// stores and the `publisher` the fan-out reaches.
    pub fn event_ingress(&self, publisher: Option<Arc<dyn EventPublisher>>) -> Arc<EventAdmission> {
        Arc::new(EventAdmission::new(
            Some(self.turns.clone()),
            Some(self.events.clone()),
            publisher,
        ))
    }

    /// The `tags` and `task_tags` tables, as a host reads and writes them.
    pub fn tags(&self) -> Arc<Tags> {
        Arc::new(Tags::new(self.tags.clone()))
    }

    /// The `tags` table behind the published `TagResolver` seam.
    pub fn tag_catalog(&self) -> Arc<crate::ports::TagCatalog> {
        Arc::new(crate::ports::TagCatalog::new(self.tags.clone()))
    }

    /// The `feedback` table, as a host reads and writes it.
    pub fn feedback(&self) -> Arc<FeedbackNotes> {
        Arc::new(FeedbackNotes::new(self.feedback.clone()))
    }

    /// The guided-review tables, as a host reads and writes them.
    pub fn review_guides(&self) -> Arc<ReviewGuides> {
        Arc::new(ReviewGuides::new(self.review_guides.clone()))
    }

    /// The `task_artifacts` table and its objects in `s3`, with response URLs
    /// under `public_url` and page tokens signed with `signing_key`.
    pub fn artifacts(
        &self,
        s3: Option<moatless_s3::S3Storage>,
        s3_config: Arc<moatless_core::config::S3Config>,
        public_url: &str,
        signing_key: Option<String>,
    ) -> Arc<Artifacts> {
        Arc::new(Artifacts::new(
            self.artifacts.clone(),
            s3,
            s3_config,
            public_url,
            signing_key,
        ))
    }

    /// The unclaimed-user-message predicate the desired-state reconciler reads.
    pub fn pending_work(&self) -> Arc<crate::ports::PendingWork> {
        Arc::new(crate::ports::PendingWork::new(self.db.clone()))
    }

    /// The `conversation_activities` table behind the published `ActivitySource` seam.
    pub fn activities(&self) -> Arc<crate::ports::Activities> {
        Arc::new(crate::ports::Activities::new(self.db.clone()))
    }

    /// A Task's transcript as a reader opens it.
    pub fn transcript_reads(&self) -> Arc<crate::conversation::transcript::TranscriptReads> {
        Arc::new(crate::conversation::transcript::TranscriptReads::new(
            self.db.clone(),
        ))
    }

    /// Per-model token and cost totals over the Turns a viewer may read.
    pub fn turn_usage(&self) -> Arc<crate::task_turn::usage::TurnUsage> {
        Arc::new(crate::task_turn::usage::TurnUsage::new(self.db.clone()))
    }

    /// The Codex in-flight guard and sandbox fan-out over a pure credential
    /// service.
    ///
    /// Built before [`TaskGraph::assemble`] rather than by it: the preflight
    /// port `assemble` takes reads this, so the host needs it in hand first.
    pub fn credential_sync(
        &self,
        credentials: Arc<CredentialService>,
        broadcast: Arc<dyn CredentialBroadcast>,
    ) -> Arc<CredentialSync> {
        Arc::new(CredentialSync::new(
            credentials,
            self.tasks.clone(),
            broadcast,
        ))
    }

    /// Assemble the workflow services over the graph's stores and the foreign
    /// ports they read. Consumes the graph: nothing re-wires a service after
    /// this returns.
    pub fn assemble(self, inputs: TaskWorkflowInputs) -> TaskOperations {
        let TaskWorkflowInputs {
            repository_reader,
            workspace_resolver,
            user_display,
            task_bindings,
            sandbox,
            starter,
            joiner,
            preflight,
            events,
            files,
            self_echo,
            config,
        } = inputs;

        let trajectories = Arc::new(Trajectories::new(PgTrajectoryDao(self.db.clone())));
        TaskOperations {
            task_creation: Arc::new(TaskCreationService::new(
                self.tasks.clone(),
                self.tags.clone(),
                self.relations.clone(),
                self.repositories.clone(),
                self.forks,
                repository_reader.clone(),
                workspace_resolver.clone(),
                events.clone(),
                starter,
                joiner,
                preflight,
            )),
            message_send: Arc::new(MessageSendService::new(
                self.messages.clone(),
                self.tasks.clone(),
                self.turns.clone(),
                files,
                self_echo,
                crate::inbox::InboxService::new(self.db.clone()),
            )),
            task_closer: Arc::new(TaskCloser::new(
                self.tasks.clone(),
                self.relations.clone(),
                config,
                sandbox,
                task_bindings.clone(),
                events,
            )),
            task_presenter: Arc::new(TaskPresenter::new(
                self.tags.clone(),
                repository_reader,
                workspace_resolver,
                user_display,
                task_bindings,
                self.relations,
                self.tasks.clone(),
            )),
            task_switch: Arc::new(TaskSwitchService::new(
                self.db.clone(),
                self.tasks.clone(),
                self.messages.clone(),
                trajectories.clone(),
            )),
            task_rewind: Arc::new(TaskRewindService::new(
                self.db,
                self.tasks.clone(),
                self.turns.clone(),
                self.messages.clone(),
                trajectories.clone(),
            )),
            task_fork: Arc::new(TaskForkService::new(
                self.tasks,
                self.turns,
                self.messages,
                self.tags,
                trajectories,
            )),
        }
    }
}

/// The foreign ports the tasks band's workflow services read, supplied by the
/// host that owns them.
pub struct TaskWorkflowInputs {
    pub repository_reader: Arc<dyn RepositoryReader>,
    pub workspace_resolver: Arc<dyn WorkspaceResolver>,
    pub user_display: Arc<dyn UserDisplay>,
    pub task_bindings: Arc<dyn TaskBindings>,
    /// Stop and archive-retention control, for the close path.
    pub sandbox: Arc<dyn SandboxLifecycle>,
    /// Provisioning, for the create path.
    pub starter: Arc<dyn SandboxStarter>,
    pub joiner: Arc<dyn SandboxJoiner>,
    pub preflight: Arc<dyn PreflightPort>,
    pub events: Option<Arc<dyn EventPublisher>>,
    pub files: Arc<Files>,
    pub self_echo: Arc<dyn SelfEchoGuard>,
    pub config: Arc<Config>,
}

/// The workflow services [`TaskGraph::assemble`] produces.
pub struct TaskOperations {
    pub task_creation: Arc<TaskCreationService>,
    pub message_send: Arc<MessageSendService>,
    pub task_closer: Arc<TaskCloser>,
    pub task_presenter: Arc<TaskPresenter>,
    pub task_fork: Arc<TaskForkService>,
    pub task_rewind: Arc<TaskRewindService>,
    pub task_switch: Arc<TaskSwitchService>,
}
