//! Docker Compose installation commands (`moat docker ...`).
//!
//! Guides users through preflight → init → install → verify for running
//! Moatless locally via Docker Compose. Shells out to `docker` / `docker
//! compose`; the compose assets (`docker-compose.yml`, `.env.example`,
//! `nginx.conf`) are embedded in the binary and seeded into the work dir on
//! `init` (see `docker_compose::INSTALL_ASSETS`).

mod args;
mod dispatch;
mod docker_compose;
mod env;
mod env_file;
mod init;
mod install;
mod logs;
mod preflight;
mod status;
mod uninstall;

pub use args::{
    DockerAction, EnvAction, EnvListArgs, EnvSetArgs, EnvUnsetArgs, InitArgs, InstallArgs,
    LogsArgs, PreflightArgs, StatusArgs, UninstallArgs,
};
pub use dispatch::run;
