//! gRPC server implementing `BackendSandboxService`.
//!
//! Receives events from sandbox pods via gRPC and serves pending messages.

mod authorize;
mod commands;
mod credentials;
mod dispatch;
mod events;
mod git_push;
mod messages;
mod plugins;
mod state;
mod trajectory;
mod transcripts;
mod uploads;
mod workspace_sync;

#[cfg(test)]
mod test_support;

pub use authorize::SandboxAuthorizer;
pub use state::BackendSandboxServiceImpl;
pub use state::HandlerDbPorts;
pub use state::IngressPorts;
