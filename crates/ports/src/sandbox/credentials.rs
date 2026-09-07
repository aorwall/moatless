use async_trait::async_trait;
use moatless_types::sandbox::SandboxError;

/// Push one rotated credential to a task's running sandbox.
///
/// The narrow half of the connector the credential fan-out needs. It is its own
/// port so the tasks band, which does nothing else with a sandbox client, names
/// no gRPC type: the sandbox band implements it over the connector.
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
/// The refresh itself needs the tasks band — it yields to a running sandbox's
/// own refresher rather than racing it for the single-use refresh token, and it
/// fans a rotated blob out to every open codex sandbox in scope. This port
/// inverts that edge so the sandbox band, which only needs the resulting
/// `auth.json`, names no tasks-band type.
///
/// `Ok(None)` is "no codex credential at any scope", which is an ordinary
/// outcome: the caller leaves the slot empty and lets a scoped secret fill it.
/// `Err` is a failure to report.
#[async_trait]
pub trait CodexCredentials: Send + Sync {
    async fn ensure_fresh_codex_for(
        &self,
        user_id: &str,
        except_task_id: Option<&str>,
    ) -> Result<Option<String>, String>;
}
