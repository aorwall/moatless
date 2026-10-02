use clap::{Parser, Subcommand};
use moatless_cli::commands;

/// Package version and the CI commit, or a local-build marker.
fn version() -> String {
    let commit =
        option_env!("GITHUB_SHA").map_or("local build", |sha| sha.get(..12).unwrap_or(sha));
    format!("{} ({commit})", env!("CARGO_PKG_VERSION"))
}

#[derive(Parser, Clone)]
pub(super) struct GlobalOpts {
    /// Output raw JSON instead of human-readable tables
    #[arg(long, global = true)]
    pub(super) json: bool,

    /// Show HTTP request details for debugging
    #[arg(long, global = true)]
    pub(super) verbose: bool,

    /// Seconds to wait for a response, overriding the 30-second default. Raise
    /// it for commands whose endpoint provisions or removes a sandbox before it
    /// answers.
    #[arg(long, global = true)]
    pub(super) timeout: Option<u64>,
}

// ---------------------------------------------------------------------------
// Commands available in BOTH modes
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub(super) enum SharedCommands {
    /// Show current user info
    Whoami,
    /// Show current context (API URL, auth mode, profile)
    Status,
    /// Task management
    Tasks {
        #[command(subcommand)]
        action: commands::tasks::TaskAction,
    },
    /// Agent harness and model catalog
    Agents {
        #[command(subcommand)]
        action: commands::agents::AgentAction,
    },
    /// Repository management
    Repos {
        #[command(subcommand)]
        action: Box<commands::repos::RepoAction>,
    },
    /// Workspace management
    Workspaces {
        #[command(subcommand)]
        action: commands::workspaces::WorkspaceAction,
    },
    /// Adapter connection management
    Adapters {
        #[command(subcommand)]
        action: commands::adapters::AdaptersAction,
    },
    /// Linear issue and comment operations via the workspace OAuth token
    Linear {
        #[command(subcommand)]
        action: Box<commands::linear::LinearAction>,
    },
    /// GitHub operations — `pr create`, `pr comment`, and `issue comment` are
    /// proxied through the backend (typed task binding + webhook dedup);
    /// everything else passes through to the local `gh` CLI verbatim.
    Gh {
        /// Arguments as you would pass them to `gh` (e.g. `pr create --title
        /// …`)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Loop management (recurring agent work fired by subscriptions or
    /// schedules)
    Loops {
        #[command(subcommand)]
        action: Box<commands::loops::LoopsAction>,
    },
    /// Task binding management
    Bindings {
        #[command(subcommand)]
        action: Box<commands::adapters::BindingAction>,
    },
    /// Make a raw API request and report the status alongside the body
    Api(commands::api::ApiArgs),
    /// Task event history
    Events {
        #[command(subcommand)]
        action: commands::events::EventAction,
    },
    /// Team management
    Teams {
        #[command(subcommand)]
        action: commands::teams::TeamAction,
    },
    /// Plugin management
    Plugins {
        #[command(subcommand)]
        action: commands::plugins::PluginAction,
    },
    /// Provider settings (GitHub PAT, etc.)
    Settings {
        #[command(subcommand)]
        action: commands::settings::SettingsAction,
    },
    /// Secret management
    #[command(name = "secret")]
    Secrets {
        #[command(subcommand)]
        action: commands::secrets::SecretAction,
    },
    /// List tags
    Tags,
    /// Kubernetes installation workflow (preflight, init, secrets, install,
    /// status)
    K8s {
        #[command(subcommand)]
        action: commands::k8s::K8sAction,
    },
    /// Docker Compose installation workflow (preflight, init, install, status,
    /// logs)
    Docker {
        #[command(subcommand)]
        action: commands::docker::DockerAction,
    },
    /// List users and manage their provider identity mappings
    User {
        #[command(subcommand)]
        action: commands::users::UserAction,
    },
    /// Bot user management — provision users backed by a registered GitHub App.
    Bot {
        #[command(subcommand)]
        action: commands::bots::BotAction,
    },
    /// GitHub App administration — register, rotate a signing key, remove.
    #[command(name = "github-app")]
    GithubApp {
        #[command(subcommand)]
        action: commands::github_apps::GitHubAppAction,
    },
    /// Provider app administration — register the Forgejo, Slack, Telegram and
    /// Linear credentials this deployment connects with.
    App {
        #[command(subcommand)]
        action: commands::provider_apps::ProviderAppAction,
    },
    /// Send a message — drive a task, DM a user, or pass through an adapter
    /// destination (Slack channel/DM/thread, Linear issue, ...). Replies and
    /// emoji reactions to specific messages live under `moat messages`.
    Send(Box<commands::send::SendArgs>),
    /// Message management — list, reply, react.
    Messages {
        #[command(subcommand)]
        action: commands::messages::MessageAction,
    },
    /// Record feedback about a task to be used for improving moatless logic,
    /// agent skills, or tooling — or, with `list`, read what has been recorded
    Feedback(commands::feedback::FeedbackInvocation),
    /// Cheap multiple-choice decisions (routing, labelling, triage) from the
    /// backend's decision provider
    Decide {
        #[command(subcommand)]
        action: commands::decide::DecideAction,
    },
    /// Publish HTML pages from a task so they outlive its sandbox, and list
    /// what has been published
    Artifact {
        #[command(subcommand)]
        action: commands::artifact::ArtifactAction,
    },
}

// ---------------------------------------------------------------------------
// Sandbox mode CLI (inside a pod — no login/logout, adds task shortcuts)
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "moat", about = "Moatless sandbox CLI", version = version())]
pub(super) struct SandboxCli {
    #[command(subcommand)]
    pub(super) command: SandboxCommands,

    #[command(flatten)]
    pub(super) opts: GlobalOpts,
}

#[derive(Subcommand)]
pub(super) enum SandboxCommands {
    #[command(flatten)]
    Shared(Box<SharedCommands>),

    /// Set the name of the current task
    SetName {
        /// The new task name
        name: String,
    },

    /// Set tags on the current task (replaces existing tags)
    SetTags {
        /// Tag names (space-separated)
        tags: Vec<String>,
    },

    /// Run a command that outlives one Bash tool call. The sandbox owns the
    /// process, so it stays awake until the command finishes and wakes you when
    /// it does.
    Cmd {
        #[command(subcommand)]
        action: commands::cmd::CmdAction,
    },
}

// ---------------------------------------------------------------------------
// Standalone mode CLI (human user — has login/logout)
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "moat", about = "Moatless CLI", version = version())]
pub(super) struct StandaloneCli {
    #[command(subcommand)]
    pub(super) command: StandaloneCommands,

    #[command(flatten)]
    pub(super) opts: GlobalOpts,
}

#[derive(Subcommand)]
pub(super) enum StandaloneCommands {
    #[command(flatten)]
    Shared(Box<SharedCommands>),

    /// Authenticate with the Moatless API.
    ///
    /// Asks the server which flow it runs (`GET /api/v1/auth/mode`): local
    /// password auth (prompts, or `MOATLESS_EMAIL`/`MOATLESS_PASSWORD`
    /// non-interactively) or an OAuth provider's browser-based flow.
    Login {
        /// URL to authenticate against. Defaults to the configured API URL (env
        /// `MOATLESS_API_URL` or the default profile).
        #[arg(long)]
        url: Option<String>,
    },
    /// Remove saved credentials
    Logout,
}
