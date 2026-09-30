use super::{DockerAction, env, init, install, logs, preflight, status, uninstall};
use crate::client::ClientError;
use moatless_types::http::HttpTimeouts;

pub async fn run(
    action: DockerAction,
    json: bool,
    local_stack: HttpTimeouts,
) -> Result<(), ClientError> {
    match action {
        DockerAction::Preflight(args) => preflight::run(args, json).await,
        DockerAction::Init(args) => init::run(args, json).await,
        DockerAction::Install(args) => install::run(args, json).await,
        DockerAction::Status(args) => status::run(args, json, local_stack).await,
        DockerAction::Logs(args) => logs::run(args, json).await,
        DockerAction::Uninstall(args) => uninstall::run(args, json).await,
        DockerAction::Env(action) => env::run(action, json).await,
    }
}
