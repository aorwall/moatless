use codex_app_server_protocol::HookCompletedNotification;
use codex_app_server_protocol::HookStartedNotification;

/// Read-only observer for hook events.
///
/// Unlike the Claude SDK's `HookHandler` (which can approve/deny/inject messages),
/// Codex hooks run server-side and the SDK only receives notifications about
/// their start and completion.
pub trait HookObserver: Send + Sync {
    fn on_hook_started(&self, params: &HookStartedNotification);
    fn on_hook_completed(&self, params: &HookCompletedNotification);
}
