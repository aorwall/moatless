//! Kubernetes installation commands (`moat k8s ...`).
//!
//! Guides users through preflight → init → secrets → install → verify for
//! deploying Moatless Vibe into a Kubernetes cluster. Shells out to the
//! locally-installed `kubectl` and `helm` binaries.

mod args;
mod cluster;
mod dispatch;
mod init;
mod install;
mod preflight;
mod secrets;
mod status;
mod uninstall;

pub use args::{
    ComputeMode, InitArgs, InstallArgs, K8sAction, PreflightArgs, Provider, StatusArgs,
    StorageMode, UninstallArgs, UpgradeArgs,
};
pub use dispatch::run;
