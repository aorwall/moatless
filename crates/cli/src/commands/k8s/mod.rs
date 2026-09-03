//! Kubernetes installation commands (`moat k8s ...`).
//!
//! Guides users through preflight → init → secrets → install → verify for
//! deploying Moatless Vibe into a Kubernetes cluster. Shells out to the
//! locally-installed `kubectl` and `helm` binaries.

use clap::{Subcommand, ValueEnum};

use crate::client::ClientError;

mod cluster;
mod init;
mod install;
mod preflight;
mod secrets;
mod status;
mod uninstall;

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// Azure Kubernetes Service
    Aks,
    /// Amazon Elastic Kubernetes Service
    Eks,
    /// Unknown / generic (no provider presets applied)
    Generic,
}

#[derive(Copy, Clone, Debug, Default, ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputeMode {
    #[default]
    Standard,
    Fargate,
}

#[derive(Copy, Clone, Debug, Default, ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageMode {
    #[default]
    Ebs,
    Ephemeral,
    S3,
    Node,
}

impl StorageMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ebs => "ebs",
            Self::Ephemeral => "ephemeral",
            Self::S3 => "s3",
            Self::Node => "node",
        }
    }
}

impl Provider {
    fn as_str(self) -> &'static str {
        match self {
            Provider::Aks => "aks",
            Provider::Eks => "eks",
            Provider::Generic => "generic",
        }
    }
}

#[derive(Subcommand)]
pub enum K8sAction {
    /// Inspect the current cluster and report readiness for an install
    Preflight(PreflightArgs),

    /// Generate a `values.local.yaml` interactively
    Init(InitArgs),

    /// Manage the cluster Secret holding app credentials
    Secrets {
        #[command(subcommand)]
        action: secrets::SecretsAction,
    },

    /// Install or upgrade the Moatless Vibe Helm release
    Install(InstallArgs),

    /// Upgrade an existing release to a new image tag
    Upgrade(UpgradeArgs),

    /// Report health of the current installation
    Status(StatusArgs),

    /// Uninstall the Helm release (does not drop PVCs unless --purge is set)
    Uninstall(UninstallArgs),
}

#[derive(clap::Args)]
pub struct PreflightArgs {
    /// Namespace to inspect (defaults to `vibe`)
    #[arg(long, short = 'n')]
    pub namespace: Option<String>,

    /// Kubernetes context to inspect without changing the current context
    #[arg(long)]
    pub context: Option<String>,

    /// Kubernetes compute substrate
    #[arg(long, value_enum, default_value_t)]
    pub compute_mode: ComputeMode,

    /// Sandbox workspace storage mode
    #[arg(long, value_enum, default_value_t)]
    pub storage_mode: StorageMode,
}

#[derive(clap::Args)]
pub struct InitArgs {
    /// Kubernetes context expected by the infrastructure handoff
    #[arg(long)]
    pub context: Option<String>,

    /// Target provider preset (auto-detected if omitted)
    #[arg(long, value_enum)]
    pub provider: Option<Provider>,

    /// Infrastructure handoff emitted by the cloud provisioning skill
    #[arg(long)]
    pub infrastructure_handoff: Option<String>,

    /// Output file (defaults to `values.local.yaml` in cwd)
    #[arg(long, short = 'o')]
    pub output: Option<String>,

    /// Main application domain (e.g. moatless.example.com)
    #[arg(long)]
    pub domain: Option<String>,

    /// Wildcard preview domain (e.g. preview.example.com)
    #[arg(long)]
    pub proxy_domain: Option<String>,

    /// Namespace to install into
    #[arg(long, short = 'n')]
    pub namespace: Option<String>,

    /// Namespace used for dynamically created sandbox pods
    #[arg(long)]
    pub sandbox_namespace: Option<String>,

    /// Image tag (defaults to latest)
    #[arg(long)]
    pub tag: Option<String>,

    /// Auth provider: local, `github_app`, auth0
    #[arg(long)]
    pub auth: Option<String>,

    /// Overwrite an existing output file without prompting
    #[arg(long)]
    pub force: bool,
}

#[derive(clap::Args)]
pub struct InstallArgs {
    /// Kubernetes context to target without changing the current context
    #[arg(long)]
    pub context: Option<String>,

    /// Helm release name
    #[arg(long, default_value = cluster::DEFAULT_RELEASE)]
    pub release: String,

    /// Namespace to install into
    #[arg(long, short = 'n', default_value = cluster::DEFAULT_NAMESPACE)]
    pub namespace: String,

    /// Path to the Helm chart
    #[arg(long, default_value = cluster::DEFAULT_CHART_PATH)]
    pub chart: String,

    /// Values file (defaults to `values.local.yaml`)
    #[arg(long, short = 'f', default_value = cluster::DEFAULT_VALUES_FILE)]
    pub values: String,

    /// Render only — don't apply to the cluster
    #[arg(long)]
    pub dry_run: bool,

    /// Skip `helm dependency update`
    #[arg(long)]
    pub skip_deps: bool,

    /// Wait for the rollout to complete (adds --wait --timeout)
    #[arg(long)]
    pub wait: bool,
}

#[derive(clap::Args)]
pub struct UpgradeArgs {
    /// Kubernetes context to target without changing the current context
    #[arg(long)]
    pub context: Option<String>,

    /// Helm release name
    #[arg(long, default_value = cluster::DEFAULT_RELEASE)]
    pub release: String,

    /// Namespace
    #[arg(long, short = 'n', default_value = cluster::DEFAULT_NAMESPACE)]
    pub namespace: String,

    /// New image tag to roll out
    #[arg(long)]
    pub tag: String,
}

#[derive(clap::Args)]
pub struct StatusArgs {
    /// Kubernetes context to inspect without changing the current context
    #[arg(long)]
    pub context: Option<String>,

    /// Namespace to inspect
    #[arg(long, short = 'n', default_value = cluster::DEFAULT_NAMESPACE)]
    pub namespace: String,

    /// Helm release name
    #[arg(long, default_value = cluster::DEFAULT_RELEASE)]
    pub release: String,
}

#[derive(clap::Args)]
pub struct UninstallArgs {
    /// Kubernetes context to target without changing the current context
    #[arg(long)]
    pub context: Option<String>,

    /// Helm release name
    #[arg(long, default_value = cluster::DEFAULT_RELEASE)]
    pub release: String,

    /// Namespace
    #[arg(long, short = 'n', default_value = cluster::DEFAULT_NAMESPACE)]
    pub namespace: String,

    /// Also delete the `vibe-secrets` Secret and orphaned PVCs
    #[arg(long)]
    pub purge: bool,
}

pub async fn run(action: K8sAction, json: bool) -> Result<(), ClientError> {
    match action {
        K8sAction::Preflight(args) => preflight::run(args, json).await,
        K8sAction::Init(args) => init::run(args, json).await,
        K8sAction::Secrets { action } => secrets::run(action, json).await,
        K8sAction::Install(args) => install::run(args, json).await,
        K8sAction::Upgrade(args) => install::upgrade(args, json).await,
        K8sAction::Status(args) => status::run(args, json).await,
        K8sAction::Uninstall(args) => uninstall::run(args, json).await,
    }
}
