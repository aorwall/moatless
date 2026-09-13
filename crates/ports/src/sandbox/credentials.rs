use async_trait::async_trait;
use moatless_types::sandbox::SandboxError;

/// Push one rotated credential to a task's running sandbox.
///
/// Its own port so the tasks band, which does nothing else with a sandbox
/// client, names no gRPC type.
#[async_trait]
pub trait CredentialBroadcast: Send + Sync {
    async fn refresh_credential(
        &self,
        task_id: &str,
        key: &str,
        value: &str,
    ) -> Result<(), SandboxError>;
}

/// Just-in-time Codex credential refresh for the identity a sandbox acts as.
///
/// Inverts the tasks-band edge — yields to a running sandbox's own refresher
/// and fans a rotated blob out to every open codex sandbox — so this band names
/// no tasks-band type. `Ok(None)` is no credential at any scope; `Err` is a
/// failure to report.
#[async_trait]
pub trait CodexCredentials: Send + Sync {
    async fn ensure_fresh_codex_for(
        &self,
        user_id: &str,
        except_task_id: Option<&str>,
    ) -> Result<Option<String>, String>;
}
