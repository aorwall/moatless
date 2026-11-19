use crate::change_watch::ChangeWatch;
use crate::config::AgentModelDefaults;
use crate::config::UiRpcConfig;
use crate::observability;
use crate::ports;
use crate::preview::PreviewState;
use crate::reads::T3Reads;
use crate::sandbox::SandboxState;
use crate::scripts::ScriptsState;
use crate::subtasks::SubtasksState;
use crate::terminal;
use moatless_core::change_feed::listing::EnvironmentIdentity;
use moatless_identity::auth::pairing::DevicePairing;
use moatless_ports::sandbox::WorkspaceSnapshotSource;
use sandbox_proto::client::SandboxConnector;
use std::sync::Arc;

pub type UiRpcDbPool = sqlx::PgPool;

#[derive(Clone)]
pub(crate) struct UiRpcState {
    pub(crate) rpc: Arc<UiRpcConfig>,
    /// The database this process started against, and therefore the database
    /// every position it has handed out was allocated in. Compared against what
    /// the database says it is on each resume; see
    /// [`resume`](moatless_core::change_feed::listing::resume).
    pub(crate) environment: Arc<EnvironmentIdentity>,
    pub(crate) session_cookie_name: Arc<str>,
    /// This deployment's own web address, for the one link this module builds —
    /// a Task origin's link back into the Moatless UI.
    pub(crate) frontend_url: Arc<str>,
    /// Signs WebSocket tickets. Absent when the deployment configures no JWT
    /// secret, which is the same condition under which session cookies do not
    /// work either; the ticket route then answers that it cannot issue one.
    pub(crate) session_jwt_secret: Option<Arc<str>>,
    /// Turns a pairing code into the token a client on another device keeps.
    /// The issuing half lives with the rest of the account routes.
    pub(crate) pairing: Arc<DevicePairing>,
    /// Nudges open Authorized clients lists after a pairing changes.
    pub(crate) access_changes: crate::access::AccessChanges,
    /// Every scoped read this surface answers from.
    pub(crate) reads: T3Reads,
    /// The change counters this replica's subscriptions wait on, read by one
    /// [`ChangePoller`](crate::change_watch::ChangePoller).
    pub(crate) changes: ChangeWatch,
    /// How a conversation's files are reached. The only piece of Moatless
    /// behind this module that is not a database read: a checkout exists inside
    /// its Sandbox and nowhere else.
    pub(crate) sandboxes: Arc<dyn SandboxConnector>,
    /// The workspace as S3 last stored it, read when a conversation's Sandbox
    /// is not running. `None` when the deployment stores no snapshots (no S3
    /// bucket), in which case a stopped Task's files are unavailable as before.
    pub(crate) snapshots: Option<Arc<dyn WorkspaceSnapshotSource>>,
    /// Running a Workspace's declared Scripts: reads the scripts and publishes
    /// a script's port. The terminal it runs in is reached through `reads`,
    /// `sandboxes` and `terminal`, the same handles `terminal.open` uses.
    pub(crate) scripts: ScriptsState,
    /// What a Task's Sandbox lifecycle is doing, and lifecycle commands.
    pub(crate) sandbox: SandboxState,
    /// The Tasks a Task spawned. A second fleet beside its subagents, and not
    /// the same kind of thing: each one is a Thread of its own.
    pub(crate) subtasks: SubtasksState,
    /// The browser tabs open on a Task, and the fan-out that tells this
    /// replica's subscribers when one changes. A tab the environment opens
    /// reaches the other replicas over `preview_relay`.
    pub(crate) preview: PreviewState,
    pub(crate) model_defaults: Arc<AgentModelDefaults>,
    /// The agent catalog, as this deployment can actually run it. Read on every
    /// config request because credentials come and go without a restart.
    pub(crate) agents: Arc<dyn ports::AgentCatalogSource>,
    /// A repository's branches, read from its git host, for a draft thread's
    /// branch picker before any Sandbox exists.
    pub(crate) branches: Arc<dyn ports::RepositoryBranchSource>,
    /// The skills a Workspace offers, for the composer's `/` menu. Read per
    /// `cwd` on demand rather than with the config: the listing reaches a git
    /// host, and a connection only ever needs the threads it opens.
    pub(crate) skills: Arc<dyn ports::SkillCatalog>,
    /// The pull request a Task is bound to, and reference resolution for the
    /// attach-PR dialog.
    pub(crate) pull_requests: Arc<dyn ports::BoundPullRequestSource>,
    /// Linked pull requests read from their git host, and the Task a guided
    /// review starts.
    pub(crate) pull_request_reviews: Arc<dyn ports::PullRequestReviewSource>,
    /// [`crate::compose::UiRpcDependencies::issues`].
    pub(crate) issues: Arc<dyn ports::IssueTrackerSource>,
    /// Stored guided reviews and the runs writing them.
    pub(crate) review_guides: Arc<moatless_tasks::review_guide::operations::ReviewGuides>,
    /// The viewer's usable GitHub credential, for the Source Control panel's
    /// auth badge. Read per request because credentials come and go without a
    /// restart.
    pub(crate) source_control_auth: Arc<dyn ports::SourceControlAuthSource>,
    /// Carries out the commands the T3 client dispatches. Not this module's
    /// only write: a git action reaches a Sandbox through `sandboxes`.
    pub(crate) commands: Arc<dyn ports::ThreadCommandSink>,
    /// Where a chat attachment's bytes go, for the upload route and for
    /// `attachments.delete`.
    pub(crate) attachments: Arc<dyn ports::AttachmentStore>,
    /// Who may read a Task's Artifacts, for `artifacts.setVisibility`.
    pub(crate) artifacts: Arc<dyn ports::ArtifactVisibilitySetter>,
    pub(crate) secret_requests: Arc<dyn ports::SecretRequestAnswerer>,
    /// Token and cost totals behind `server.getUsageSummary`.
    pub(crate) usage: Arc<dyn ports::UsageReporter>,
    /// The settings each viewer saves for themselves.
    pub(crate) viewer_settings: Arc<dyn ports::ViewerSettingsStore>,
    /// In-process fan-out for terminal metadata and events observed through
    /// this backend instance.
    pub(crate) terminal: terminal::TerminalHub,
    /// Where the T3 client's own spans are forwarded — see [`observability`].
    pub(crate) client_traces: observability::ClientTraceForwarder,
    /// Quota read off a `CLIProxyAPI` hub, when the deployment configures one.
    pub(crate) usage_limit_sources: crate::usage_limit_sources::UsageLimitSources,
}

/// What this deployment is, for the parts of a thread projection not read out
/// of the conversation.
///
/// The agent settings a thread reports where it states none of its own, and the
/// address a link back into Moatless is built from.
pub struct Deployment<'a> {
    pub model_defaults: &'a AgentModelDefaults,
    pub frontend_url: &'a str,
}

impl UiRpcState {
    pub(crate) fn deployment(&self) -> Deployment<'_> {
        Deployment {
            model_defaults: &self.model_defaults,
            frontend_url: &self.frontend_url,
        }
    }

    /// Where a conversation's files are read from: its Sandbox, and the S3
    /// snapshot once none is running.
    pub(crate) fn file_sources(&self) -> crate::checkout::Sources<'_> {
        crate::checkout::Sources {
            sandboxes: self.sandboxes.as_ref(),
            snapshots: self.snapshots.as_deref(),
        }
    }
}
