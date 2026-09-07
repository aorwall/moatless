use super::{K8sAction, init, install, preflight, secrets, status, uninstall};
use crate::client::ClientError;

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
