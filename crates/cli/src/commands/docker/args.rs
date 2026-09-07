use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum DockerAction {
    /// Inspect the host and report readiness for a Docker Compose install
    Preflight(PreflightArgs),

    /// Seed install assets into a work dir and bootstrap `.env`
    Init(InitArgs),

    /// Start the stack (`docker compose up -d`)
    Install(InstallArgs),

    /// Report health of a running installation
    Status(StatusArgs),

    /// Stream logs from the running stack
    Logs(LogsArgs),

    /// Stop the stack (`docker compose down`); `--purge` drops volumes
    Uninstall(UninstallArgs),

    /// Inspect or update keys in `<compose-dir>/.env`
    #[command(subcommand)]
    Env(EnvAction),
}

#[derive(Subcommand)]
pub enum EnvAction {
    /// Write or replace a key in `.env` (creates `.env` from `.env.example` if missing)
    Set(EnvSetArgs),

    /// Remove a key from `.env` entirely. Distinct from `set KEY ""` — the next
    /// `init` will treat the key as missing and regenerate auto-generated
    /// secrets (e.g. `TASK_TOKEN_SECRET`, `SECRETS_ENCRYPTION_KEY`).
    Unset(EnvUnsetArgs),

    /// Show all keys in `.env` with status (set / placeholder / empty); secrets are masked
    List(EnvListArgs),
}

#[derive(clap::Args)]
pub struct EnvSetArgs {
    /// Explicit compose work dir
    #[arg(long)]
    pub compose_dir: Option<PathBuf>,

    /// Key name, e.g. `CLAUDE_CODE_OAUTH_TOKEN`
    pub key: String,

    /// Value to write (secret values are not echoed to stdout)
    pub value: String,
}

#[derive(clap::Args)]
pub struct EnvUnsetArgs {
    /// Explicit compose work dir
    #[arg(long)]
    pub compose_dir: Option<PathBuf>,

    /// Key name to remove from `.env`
    pub key: String,
}

#[derive(clap::Args)]
pub struct EnvListArgs {
    /// Explicit compose work dir
    #[arg(long)]
    pub compose_dir: Option<PathBuf>,
}

#[derive(clap::Args)]
pub struct PreflightArgs {
    /// Explicit compose work dir (skips repo walk-up and cache dir fallback)
    #[arg(long)]
    pub compose_dir: Option<PathBuf>,
}

#[derive(clap::Args)]
pub struct InitArgs {
    /// Explicit compose work dir
    #[arg(long)]
    pub compose_dir: Option<PathBuf>,

    /// Optional override: seed the work dir from this directory instead of the
    /// binary's embedded copies of `docker-compose.yml`, `.env.example`, and
    /// `nginx.conf` (an alternate asset bundle)
    #[arg(long)]
    pub assets_from: Option<PathBuf>,

    /// Overwrite existing files in the work dir without prompting
    #[arg(long)]
    pub overwrite: bool,

    /// Don't prompt — report missing keys instead
    #[arg(long)]
    pub non_interactive: bool,
}

#[derive(clap::Args)]
pub struct InstallArgs {
    /// Explicit compose work dir
    #[arg(long)]
    pub compose_dir: Option<PathBuf>,

    /// Run `docker compose pull` before `up` to refresh images
    #[arg(long)]
    pub pull: bool,

    /// Block until `status` reports `all_healthy` (120s timeout)
    #[arg(long)]
    pub wait: bool,
}

#[derive(clap::Args)]
pub struct StatusArgs {
    /// Explicit compose work dir
    #[arg(long)]
    pub compose_dir: Option<PathBuf>,
}

#[derive(clap::Args)]
pub struct LogsArgs {
    /// Explicit compose work dir
    #[arg(long)]
    pub compose_dir: Option<PathBuf>,

    /// Limit to one service (backend, frontend, postgres, redis, dozzle)
    #[arg(long)]
    pub service: Option<String>,

    /// Follow log output
    #[arg(long, short = 'f')]
    pub follow: bool,

    /// Only show logs from this many lines back (e.g. `100`) or time window (e.g. `10m`)
    #[arg(long)]
    pub since: Option<String>,
}

#[derive(clap::Args)]
pub struct UninstallArgs {
    /// Explicit compose work dir
    #[arg(long)]
    pub compose_dir: Option<PathBuf>,

    /// Also drop compose volumes (wipes postgres + redis data)
    #[arg(long)]
    pub purge: bool,

    /// Also remove `$HOME/.moatless/compose/` (the managed cache dir)
    #[arg(long)]
    pub purge_cache: bool,
}
