//! What a sandbox-issued `X-User-Token` may call: deny by default, so a route
//! added later stays out of an agent's reach until it is listed here.

use axum::http::{Method, StatusCode};
use moatless_core::http_auth::SandboxReach;
use moatless_types::auth::SandboxTokenScope;
use moatless_types::user::UserRole;
use tracing::warn;

/// Who may call a route with a sandbox token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reach<'a> {
    /// Any sandbox.
    Any,
    /// A sandbox the named Task is in reach of.
    Task(&'a str),
    /// A sandbox the named message's Task is in reach of.
    Message(&'a str),
    /// A sandbox whose Workspace holds an `agentAdmin` grant, for a driver
    /// who is an administrator.
    Admin,
    /// No sandbox: the route mints a credential or changes the account.
    Never,
}

impl Reach<'_> {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Task(_) => "task",
            Self::Message(_) => "message",
            Self::Admin => "admin",
            Self::Never => "never",
        }
    }
}

/// The role a sandbox token's request runs under, or the status that refuses
/// it. A driver keeps the administrator role only while the Workspace holds
/// an `agentAdmin` grant; under [`SandboxTokenScope::Log`] nothing is refused
/// or capped, and each refusal is logged instead.
pub(super) async fn admit(
    port: &dyn SandboxReach,
    scope: SandboxTokenScope,
    method: &Method,
    path: &str,
    sandbox_id: &str,
    role: UserRole,
) -> Result<UserRole, StatusCode> {
    let enforce = scope == SandboxTokenScope::Enforce;
    let reach = reach(method, path);
    let agent_admin = role == UserRole::Admin
        && (enforce || reach != Reach::Any)
        && port.is_agent_admin(sandbox_id).await;
    let refused = match reach {
        Reach::Any => None,
        _ if agent_admin && reach != Reach::Never => None,
        Reach::Task(task_id) => {
            (!port.task_in_reach(task_id, sandbox_id).await).then_some(StatusCode::NOT_FOUND)
        }
        Reach::Message(message_id) => {
            (!port.message_in_reach(message_id, sandbox_id).await).then_some(StatusCode::NOT_FOUND)
        }
        Reach::Admin | Reach::Never => Some(StatusCode::FORBIDDEN),
    };
    if let Some(status) = refused {
        warn!(
            sandbox_id,
            %method,
            path,
            reach = reach.label(),
            enforced = enforce,
            "Sandbox token outside its reach"
        );
        if enforce {
            return Err(status);
        }
    }
    Ok(if enforce && !agent_admin {
        UserRole::User
    } else {
        role
    })
}

/// The [`Reach`] of `method` on `path`, which carries no query string.
pub(super) fn reach<'a>(method: &Method, path: &'a str) -> Reach<'a> {
    let read = matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
    let segments: Vec<&'a str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        ["api", "v1", rest @ ..] => v1(read, method, rest),
        // Status reads of any task the driver can see are open; `logs` and file
        // reads stay with the sandbox's own tasks, since they carry tool output.
        ["api", "sandbox", "v1", "tasks", _, "status" | "live-status"] if read => Reach::Any,
        ["api", "sandbox", "v1", "tasks", task, ..] => Reach::Task(task),
        ["api", "orchestration", "scripts", task, ..] => Reach::Task(task),
        // Session, client and WebSocket-ticket minting; `/ws` is UI-RPC under
        // the driver's whole identity.
        ["api", "auth", ..] | ["ws"] => Reach::Never,
        _ => Reach::Admin,
    }
}

fn v1<'a>(read: bool, method: &Method, rest: &[&'a str]) -> Reach<'a> {
    match rest {
        ["auth", "me"] if *method == Method::GET => Reach::Any,
        ["auth", ..] | ["settings", ..] | ["workspaces", _, "credentials", ..] => Reach::Never,
        ["users", .., "provider-ids"] | ["users", .., "provider-ids", _] if !read => Reach::Never,

        ["tasks"] => Reach::Any,
        ["tasks", ..] if read => Reach::Any,
        ["tasks", _, "messages"] if *method == Method::POST => Reach::Any,
        ["tasks", task, ..] => Reach::Task(task),
        ["messages", message, _, ..] => Reach::Message(message),

        ["feedback"] | ["users", _, "dm"] | ["adapters", "send"] => Reach::Any,
        [
            "decisions" | "github" | "linear" | "uploads" | "events" | "artifacts",
            ..,
        ] => Reach::Any,
        [
            "repositories" | "workspaces" | "loops" | "plugins" | "tags" | "agents" | "adapters"
            | "feature-flags",
            ..,
        ] if read => Reach::Any,
        _ => Reach::Admin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reaches task `t-own` and message `m-own`, and is an agent admin iff
    /// built with `admin`.
    struct FakeReach {
        admin: bool,
    }

    #[async_trait::async_trait]
    impl SandboxReach for FakeReach {
        async fn task_in_reach(&self, task_id: &str, _sandbox_id: &str) -> bool {
            task_id == "t-own"
        }
        async fn message_in_reach(&self, message_id: &str, _sandbox_id: &str) -> bool {
            message_id == "m-own"
        }
        async fn is_agent_admin(&self, _sandbox_id: &str) -> bool {
            self.admin
        }
    }

    async fn admit_as(
        admin: bool,
        role: UserRole,
        scope: SandboxTokenScope,
        method: Method,
        path: &str,
    ) -> Result<UserRole, StatusCode> {
        admit(&FakeReach { admin }, scope, &method, path, "s1", role).await
    }

    #[tokio::test]
    async fn enforce_caps_an_admin_driver_without_the_grant_to_user() {
        let enforce = SandboxTokenScope::Enforce;
        assert_eq!(
            admit_as(
                false,
                UserRole::Admin,
                enforce,
                Method::GET,
                "/api/v1/tasks"
            )
            .await,
            Ok(UserRole::User)
        );
        assert_eq!(
            admit_as(true, UserRole::Admin, enforce, Method::GET, "/api/v1/tasks").await,
            Ok(UserRole::Admin)
        );
        assert_eq!(
            admit_as(
                false,
                UserRole::Admin,
                enforce,
                Method::GET,
                "/api/v1/users"
            )
            .await,
            Err(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            admit_as(true, UserRole::User, enforce, Method::GET, "/api/v1/users").await,
            Err(StatusCode::FORBIDDEN),
            "the grant never raises a driver who is not an administrator"
        );
        assert_eq!(
            admit_as(true, UserRole::Admin, enforce, Method::GET, "/api/v1/users").await,
            Ok(UserRole::Admin)
        );
    }

    #[tokio::test]
    async fn enforce_refuses_writes_outside_the_sandbox_and_every_credential_route() {
        let enforce = SandboxTokenScope::Enforce;
        let user = UserRole::User;
        assert_eq!(
            admit_as(
                false,
                user,
                enforce,
                Method::POST,
                "/api/v1/tasks/t-own/messages"
            )
            .await,
            Ok(UserRole::User)
        );
        assert_eq!(
            admit_as(
                false,
                user,
                enforce,
                Method::POST,
                "/api/v1/tasks/t-other/cancel"
            )
            .await,
            Err(StatusCode::NOT_FOUND)
        );
        assert_eq!(
            admit_as(
                false,
                user,
                enforce,
                Method::POST,
                "/api/v1/messages/m-other/reply"
            )
            .await,
            Err(StatusCode::NOT_FOUND)
        );
        assert_eq!(
            admit_as(
                true,
                UserRole::Admin,
                enforce,
                Method::POST,
                "/api/v1/auth/api-keys"
            )
            .await,
            Err(StatusCode::FORBIDDEN),
            "not even an agent admin mints a credential"
        );
    }

    #[tokio::test]
    async fn enforce_admits_messaging_and_status_reads_of_any_task() {
        let enforce = SandboxTokenScope::Enforce;
        for (method, path) in [
            (Method::POST, "/api/v1/tasks/t-other/messages"),
            (Method::GET, "/api/sandbox/v1/tasks/t-other/status"),
            (Method::GET, "/api/sandbox/v1/tasks/t-other/live-status"),
        ] {
            assert_eq!(
                admit_as(false, UserRole::User, enforce, method.clone(), path).await,
                Ok(UserRole::User),
                "{method} {path}"
            );
        }
        for (method, path) in [
            (Method::GET, "/api/sandbox/v1/tasks/t-other/files/read"),
            (Method::GET, "/api/sandbox/v1/tasks/t-other/logs"),
            (Method::POST, "/api/sandbox/v1/tasks/t-other/stop"),
        ] {
            assert_eq!(
                admit_as(false, UserRole::User, enforce, method.clone(), path).await,
                Err(StatusCode::NOT_FOUND),
                "{method} {path}"
            );
        }
    }

    #[tokio::test]
    async fn log_mode_refuses_and_caps_nothing() {
        let log = SandboxTokenScope::Log;
        assert_eq!(
            admit_as(
                false,
                UserRole::Admin,
                log,
                Method::POST,
                "/api/v1/auth/api-keys"
            )
            .await,
            Ok(UserRole::Admin)
        );
        assert_eq!(
            admit_as(
                false,
                UserRole::User,
                log,
                Method::POST,
                "/api/v1/tasks/t-other/stop"
            )
            .await,
            Ok(UserRole::User)
        );
    }

    #[test]
    fn each_route_an_agent_calls_has_the_reach_its_risk_needs() {
        let cases: &[(Method, &str, Reach)] = &[
            (Method::GET, "/api/v1/auth/me", Reach::Any),
            (Method::POST, "/api/v1/tasks", Reach::Any),
            (Method::GET, "/api/v1/tasks", Reach::Any),
            (Method::GET, "/api/v1/tasks/t1/events/history", Reach::Any),
            (Method::POST, "/api/v1/tasks/t1/messages", Reach::Any),
            (
                Method::GET,
                "/api/sandbox/v1/tasks/t1/live-status",
                Reach::Any,
            ),
            (
                Method::POST,
                "/api/sandbox/v1/tasks/t1/status",
                Reach::Task("t1"),
            ),
            (Method::PUT, "/api/v1/tasks/t1/memory", Reach::Task("t1")),
            (Method::DELETE, "/api/v1/tasks/t1", Reach::Task("t1")),
            (
                Method::GET,
                "/api/sandbox/v1/tasks/t1/files/read",
                Reach::Task("t1"),
            ),
            (
                Method::POST,
                "/api/orchestration/scripts/t1/s1/run",
                Reach::Task("t1"),
            ),
            (
                Method::POST,
                "/api/v1/messages/m1/reply",
                Reach::Message("m1"),
            ),
            (
                Method::DELETE,
                "/api/v1/messages/m1/reactions/x",
                Reach::Message("m1"),
            ),
            (Method::POST, "/api/v1/feedback", Reach::Any),
            (Method::POST, "/api/v1/decisions/pick", Reach::Any),
            (Method::POST, "/api/v1/adapters/send", Reach::Any),
            (Method::POST, "/api/v1/github/repos/o/r/pulls", Reach::Any),
            (Method::POST, "/api/v1/users/alice/dm", Reach::Any),
            (Method::GET, "/api/v1/workspaces", Reach::Any),
            (Method::GET, "/api/v1/plugins/effective", Reach::Any),
            (Method::POST, "/api/v1/loops/l1/activate", Reach::Admin),
            (Method::PATCH, "/api/v1/workspaces/w1", Reach::Admin),
            (
                Method::POST,
                "/api/v1/repositories/r1/sync-config",
                Reach::Admin,
            ),
            (Method::PATCH, "/api/v1/feedback/f1/status", Reach::Admin),
            (Method::GET, "/api/v1/users", Reach::Admin),
            (Method::GET, "/api/v1/secrets", Reach::Admin),
            (Method::GET, "/api/v1/admin/settings/apps", Reach::Admin),
            (Method::GET, "/api/v1/unlisted", Reach::Admin),
            (Method::POST, "/api/v1/auth/api-keys", Reach::Never),
            (Method::POST, "/api/v1/auth/pairing-codes", Reach::Never),
            (Method::POST, "/api/v1/auth/logout-all", Reach::Never),
            (Method::PATCH, "/api/v1/auth/me", Reach::Never),
            (Method::PUT, "/api/v1/settings/github/pat", Reach::Never),
            (
                Method::GET,
                "/api/v1/workspaces/w1/credentials",
                Reach::Never,
            ),
            (
                Method::PUT,
                "/api/v1/users/alice/provider-ids/slack",
                Reach::Never,
            ),
            (
                Method::POST,
                "/api/v1/users/by-id/u1/provider-ids",
                Reach::Never,
            ),
            (Method::POST, "/api/auth/websocket-ticket", Reach::Never),
            (Method::GET, "/ws", Reach::Never),
        ];
        for (method, path, expected) in cases {
            assert_eq!(reach(method, path), *expected, "{method} {path}");
        }
    }
}
