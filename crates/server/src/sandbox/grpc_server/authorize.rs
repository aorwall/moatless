//! The one verification layer every task-scoped RPC here goes through.

use std::sync::Arc;

use tonic::{Request, Status};
use tracing::warn;

use moatless_types::access_control::ScopeContext;

use moatless_core::sandbox_jwt as jwt;
use moatless_ports::identity::CommitIdentity;
use moatless_tasks::task::operations::TaskCommands;

/// A resolved authorization to act on one task over the sandbox gRPC API.
///
/// Produced by [`SandboxAuthorizer`]; its `ctx` is what a handler runs its
/// scoped DB work as.
#[derive(Debug)]
pub(crate) struct TaskGrant {
    /// The sandbox the task is a member of.
    pub sandbox_id: String,
    /// The caller's scope context (user id + platform-admin flag).
    pub ctx: ScopeContext,
    /// The caller's login. Empty for the dev bypass.
    pub login: String,
    /// The token generation the caller's token carries, which is current.
    pub token_generation: i64,
}

/// Token-level verdict from verifying a bearer token's signature, before the
/// DB-backed task-grant resolution in [`SandboxAuthorizer::resolve_grant`].
pub(super) enum VerifiedToken {
    /// No `TASK_TOKEN_SECRET` configured: dev/test bypass. Constructed only via
    /// [`SandboxAuthorizer::dev_unauthenticated`], so the bypass has one
    /// origin.
    DevBypass,
    /// A token carrying a `taskId` claim (the post-binding norm). The claim is
    /// authoritative; the request's `task_id` must equal it.
    TaskBound {
        claims: jwt::UserTokenClaims,
        task_id: String,
    },
    /// A token carrying a `sandboxId` claim. The requested task must be a
    /// member of that sandbox.
    SandboxBound {
        claims: jwt::UserTokenClaims,
        sandbox_id: String,
    },
    /// A claim-less token accepted under the grace flag. Its request `task_id`
    /// is authorized purely by the DB control check.
    Claimless { claims: jwt::UserTokenClaims },
}

/// The single verification layer for the sandbox gRPC API.
///
/// Every RPC begins at [`authorize`](Self::authorize), or
/// [`authorize_bootstrap`](Self::authorize_bootstrap) for the one
/// `GetCredentials` exception.
pub struct SandboxAuthorizer {
    /// JWT signing secret. Enforcing mode has one; the dev bypass has none.
    secret: Option<String>,
    /// Accept claim-less tokens via the DB fallback when the grace flag is set.
    accept_claimless: bool,
    /// Task DAO, for the claim-less fallback and the control gate.
    tasks: Option<Arc<TaskCommands>>,
    /// The platform-admin lookup behind the control gate.
    users: Option<Arc<dyn CommitIdentity>>,
    /// The generation a token must carry. `None` refuses every token: only a
    /// databaseless backend has none, and it can grant no task anyway.
    generations: Option<Arc<dyn jwt::TokenGenerations>>,
}

impl SandboxAuthorizer {
    /// Production constructor: verify every token against `secret`.
    pub(crate) fn enforcing(
        secret: String,
        accept_claimless: bool,
        tasks: Option<Arc<TaskCommands>>,
        users: Option<Arc<dyn CommitIdentity>>,
        generations: Option<Arc<dyn jwt::TokenGenerations>>,
    ) -> Self {
        Self {
            secret: Some(secret),
            accept_claimless,
            tasks,
            users,
            generations,
        }
    }

    /// Dev/test constructor: no signing secret, so every request is trusted and
    /// mapped to a system-scope grant for whatever task it names.
    ///
    /// The only place the bypass is constructed; production refuses to boot
    /// without `TASK_TOKEN_SECRET` (see `Config::validate`).
    pub(crate) fn dev_unauthenticated(
        tasks: Option<Arc<TaskCommands>>,
        users: Option<Arc<dyn CommitIdentity>>,
    ) -> Self {
        Self {
            secret: None,
            accept_claimless: true,
            tasks,
            users,
            generations: None,
        }
    }

    /// Select the mode from config: `Some(secret)` → enforcing, `None` → the
    /// dev bypass. The single branch that decides authenticated-vs-bypass.
    pub fn from_config(
        secret: Option<String>,
        accept_claimless: bool,
        tasks: Option<Arc<TaskCommands>>,
        users: Option<Arc<dyn CommitIdentity>>,
        generations: Option<Arc<dyn jwt::TokenGenerations>>,
    ) -> Self {
        match secret {
            Some(secret) => Self::enforcing(secret, accept_claimless, tasks, users, generations),
            None => Self::dev_unauthenticated(tasks, users),
        }
    }

    /// Extract and verify the bearer token from request metadata.
    ///
    /// `allow_bootstrap` is set only for `GetCredentials`; the grant is
    /// resolved separately by [`resolve_grant`](Self::resolve_grant) since a
    /// streaming RPC's `task_id` arrives in-band, not yet known here.
    #[allow(clippy::result_large_err)] // reason: tonic::Status is the required gRPC error type
    pub(super) fn verify_token<T>(
        &self,
        request: &Request<T>,
        allow_bootstrap: bool,
    ) -> Result<VerifiedToken, Status> {
        let Some(secret) = self.secret.as_deref() else {
            return Ok(VerifiedToken::DevBypass);
        };

        let auth_header = request
            .metadata()
            .get("authorization")
            .ok_or_else(|| Status::unauthenticated("Missing authorization header"))?
            .to_str()
            .map_err(|_| Status::unauthenticated("Invalid authorization header"))?;
        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| Status::unauthenticated("Invalid authorization format"))?;

        let claims = if allow_bootstrap {
            jwt::verify_bootstrap_or_user_token(token, secret)
        } else {
            jwt::verify_user_auth_token(token, secret)
        }
        .map_err(|e| Status::unauthenticated(format!("Invalid token: {e}")))?;

        if let Some(sandbox_id) = claims.sandbox_id.clone() {
            return Ok(VerifiedToken::SandboxBound { claims, sandbox_id });
        }
        match claims.task_id.clone() {
            Some(task_id) => Ok(VerifiedToken::TaskBound { claims, task_id }),
            None if self.accept_claimless => Ok(VerifiedToken::Claimless { claims }),
            None => Err(Status::unauthenticated(
                "Token missing sandbox binding (sandboxId claim)",
            )),
        }
    }

    /// Resolve a verified token to a [`TaskGrant`] for `request_task_id`,
    /// refusing a token from an older generation of the granted Sandbox, or
    /// one minted for anyone but its driver.
    pub(super) async fn resolve_grant(
        &self,
        verified: VerifiedToken,
        request_task_id: &str,
    ) -> Result<TaskGrant, Status> {
        let generation = match &verified {
            VerifiedToken::DevBypass => None,
            VerifiedToken::TaskBound { claims, .. }
            | VerifiedToken::SandboxBound { claims, .. }
            | VerifiedToken::Claimless { claims } => Some(claims.generation),
        };
        let grant = self.resolve_membership(verified, request_task_id).await?;
        if let Some(generation) = generation {
            self.require_current_generation(&grant, generation).await?;
            self.require_driver(&grant).await?;
        }
        Ok(grant)
    }

    /// Require the grant's user to be the one the Sandbox's tokens are minted
    /// for: its driver, else its owner. A Sandbox with neither (a global one)
    /// leaves the write check alone to decide, as it did before drivers.
    async fn require_driver(&self, grant: &TaskGrant) -> Result<(), Status> {
        let generations = self
            .generations
            .as_deref()
            .ok_or_else(|| Status::internal("Token generations not configured"))?;
        let token_user = generations
            .token_user(&grant.sandbox_id)
            .await
            .map_err(|e| {
                warn!(error = %e, sandbox_id = %grant.sandbox_id, "Failed to read the sandbox driver");
                Status::unavailable("Failed to read the sandbox driver")
            })?;
        match token_user {
            Some(driver) if driver != grant.ctx.user_id => {
                warn!(
                    sandbox_id = %grant.sandbox_id,
                    user_id = %grant.ctx.user_id,
                    driver = %driver,
                    "Rejected sandbox gRPC call: caller is not the sandbox's driver"
                );
                Err(Status::unauthenticated("Token has been superseded"))
            }
            _ => Ok(()),
        }
    }

    async fn require_current_generation(
        &self,
        grant: &TaskGrant,
        generation: i64,
    ) -> Result<(), Status> {
        let generations = self
            .generations
            .as_deref()
            .ok_or_else(|| Status::internal("Token generations not configured"))?;
        jwt::check_generation(generations, &grant.sandbox_id, generation)
            .await
            .map_err(|e| {
                warn!(
                    error = %e,
                    sandbox_id = %grant.sandbox_id,
                    presented = generation,
                    "Rejected sandbox gRPC call: token generation not current"
                );
                match e {
                    jwt::GenerationError::Lookup(_) => {
                        Status::unavailable("Failed to read the token generation")
                    }
                    jwt::GenerationError::Stale { .. } | jwt::GenerationError::NoSandbox => {
                        Status::unauthenticated("Token has been superseded")
                    }
                }
            })
    }

    async fn resolve_membership(
        &self,
        verified: VerifiedToken,
        request_task_id: &str,
    ) -> Result<TaskGrant, Status> {
        match verified {
            VerifiedToken::DevBypass => {
                // reason: dev bypass grant — no TASK_TOKEN_SECRET configured, caller is
                // trusted (only reachable via SandboxAuthorizer::dev_unauthenticated).
                #[allow(clippy::disallowed_methods)]
                let ctx = ScopeContext::system();
                Ok(TaskGrant {
                    sandbox_id: request_task_id.to_string(),
                    ctx,
                    login: String::new(),
                    token_generation: 0,
                })
            }
            VerifiedToken::TaskBound { claims, task_id } => {
                if task_id != request_task_id {
                    warn!(
                        claim_task_id = %task_id,
                        request_task_id,
                        user_id = %claims.user_id,
                        "Rejected sandbox gRPC call: token task binding does not match request"
                    );
                    return Err(Status::permission_denied(
                        "Token is not valid for this task",
                    ));
                }
                self.resolve_write(&claims, request_task_id).await
            }
            VerifiedToken::SandboxBound { claims, sandbox_id } => {
                self.resolve_sandbox_membership(&claims, request_task_id, &sandbox_id)
                    .await
            }
            VerifiedToken::Claimless { claims } => {
                self.resolve_write(&claims, request_task_id).await
            }
        }
    }

    /// Verify + resolve in one step: the standard entrypoint for task-scoped
    /// RPCs.
    pub(super) async fn authorize<T>(
        &self,
        request: &Request<T>,
        request_task_id: &str,
    ) -> Result<TaskGrant, Status> {
        let verified = self.verify_token(request, false)?;
        self.resolve_grant(verified, request_task_id).await
    }

    /// Like [`authorize`](Self::authorize) but also accepts the bootstrap token
    /// — used by `GetCredentials` alone.
    pub(super) async fn authorize_bootstrap<T>(
        &self,
        request: &Request<T>,
        request_task_id: &str,
    ) -> Result<TaskGrant, Status> {
        let verified = self.verify_token(request, true)?;
        self.resolve_grant(verified, request_task_id).await
    }

    /// The DB-backed write gate: load the task through the caller's scoped read
    /// (an invisible task is a plain `NotFound`), then require `ctx.can_write`.
    /// [`require_driver`](Self::require_driver) narrows a public task's
    /// writers to the one whose credentials the pod holds.
    async fn resolve_write(
        &self,
        claims: &jwt::UserTokenClaims,
        task_id: &str,
    ) -> Result<TaskGrant, Status> {
        let tasks = self
            .tasks
            .as_ref()
            .ok_or_else(|| Status::internal("Tasks repository not configured"))?;

        let ctx = self.scope_context_for(claims).await?;
        let task = tasks
            .find_by_id(task_id, &ctx)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id, "resolve_write: task lookup failed");
                Status::internal("Failed to load task")
            })?
            .ok_or_else(|| Status::not_found("Task not found"))?;

        if ctx.can_write(&task.scoped_resource(), Some(task.visibility)) {
            Ok(TaskGrant {
                sandbox_id: task.sandbox_id.unwrap_or_else(|| task_id.to_string()),
                login: claims.login.clone(),
                ctx,
                token_generation: claims.generation,
            })
        } else {
            warn!(
                task_id,
                user_id = %claims.user_id,
                scope = %task.scope,
                "Rejected sandbox gRPC call: caller not authorized for task"
            );
            Err(Status::permission_denied("Not authorized for this task"))
        }
    }

    async fn resolve_sandbox_membership(
        &self,
        claims: &jwt::UserTokenClaims,
        task_id: &str,
        sandbox_id: &str,
    ) -> Result<TaskGrant, Status> {
        let tasks = self
            .tasks
            .as_ref()
            .ok_or_else(|| Status::internal("Tasks repository not configured"))?;

        let ctx = self.scope_context_for(claims).await?;
        let task = tasks
            .find_by_id(task_id, &ctx)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id, sandbox_id, "resolve_sandbox_membership: task lookup failed");
                Status::permission_denied("Sandbox membership could not be confirmed")
            })?
            .ok_or_else(|| Status::permission_denied("Task is not a member of this sandbox"))?;

        let task_sandbox_id = task.sandbox_id.as_deref().unwrap_or(&task.task_id);
        if task_sandbox_id != sandbox_id {
            warn!(
                task_id,
                task_sandbox_id,
                claim_sandbox_id = %sandbox_id,
                user_id = %claims.user_id,
                "Rejected sandbox gRPC call: token sandbox binding does not match task membership"
            );
            return Err(Status::permission_denied(
                "Token is not valid for this task's sandbox",
            ));
        }
        if !ctx.can_write(&task.scoped_resource(), Some(task.visibility)) {
            return Err(Status::permission_denied("Not authorized for this task"));
        }

        Ok(TaskGrant {
            sandbox_id: sandbox_id.to_string(),
            login: claims.login.clone(),
            ctx,
            token_generation: claims.generation,
        })
    }

    /// Hydrate the caller's `ScopeContext` from the DB. The sandbox JWT carries
    /// only `user_id`/`login`; the platform-admin flag requires one DB lookup.
    async fn scope_context_for(
        &self,
        claims: &jwt::UserTokenClaims,
    ) -> Result<ScopeContext, Status> {
        let user_uuid = claims
            .user_id
            .parse::<uuid::Uuid>()
            .map_err(|_| Status::unauthenticated("Invalid user id in token"))?;
        Ok(ScopeContext::new(
            claims.user_id.clone(),
            self.user_is_admin(&user_uuid).await,
        ))
    }

    /// Platform-admin check for the control gate. Missing repo or a lookup
    /// error is treated as "not admin" (fail closed).
    async fn user_is_admin(&self, user_id: &uuid::Uuid) -> bool {
        match self.users.as_ref() {
            Some(users) => matches!(
                users.is_platform_admin(&user_id.to_string()).await,
                Ok(true)
            ),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::claims_for;
    use super::super::test_support::service_with;
    use super::super::test_support::task_fixture;
    use super::super::test_support::tasks_returning;
    use super::super::test_support::uid;
    use super::super::test_support::users_admin;
    use super::super::test_support::users_regular;
    use super::*;

    // The scoped read's SQL filter is covered by `scope.rs` tests; these
    // exercise the control-gate composition and the token binding check.

    use moatless_types::access_control::Scope;

    use moatless_types::task::Visibility;

    #[tokio::test]
    async fn assert_task_access_allows_user_task_for_owner() {
        let svc = service_with(
            tasks_returning(Some(task_fixture(Scope::User, Some("owner")))),
            users_regular(),
        );
        assert!(
            svc.authorizer
                .resolve_write(&claims_for("owner"), "task-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn assert_task_access_denies_user_task_for_non_owner() {
        // A user-scoped task belongs only to its owner; a different caller with a
        // valid JWT must be rejected even though the token itself is genuine.
        let svc = service_with(
            tasks_returning(Some(task_fixture(Scope::User, Some("owner")))),
            users_regular(),
        );
        let err = svc
            .authorizer
            .resolve_write(&claims_for("attacker"), "task-1")
            .await
            .expect_err("non-owner must be denied");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    /// Public visibility grants write; which writer holds the pod is
    /// [`SandboxAuthorizer::require_driver`]'s question, tested below.
    #[tokio::test]
    async fn assert_task_access_public_visibility_grants_write() {
        let mut task = task_fixture(Scope::User, Some("owner"));
        task.visibility = Visibility::Public;
        let svc = service_with(tasks_returning(Some(task)), users_regular());
        assert!(
            svc.authorizer
                .resolve_write(&claims_for("stranger"), "task-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn assert_task_access_global_task_requires_admin() {
        // Global-scoped tasks are admin-created (`can_create_scope`) and
        // admin-controlled; being globally *visible* doesn't hand out control.
        let denied = service_with(
            tasks_returning(Some(task_fixture(Scope::Global, None))),
            users_regular(),
        );
        let err = denied
            .authorizer
            .resolve_write(&claims_for("stranger"), "task-1")
            .await
            .expect_err("non-admin must not control a global task");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);

        let allowed = service_with(
            tasks_returning(Some(task_fixture(Scope::Global, None))),
            users_admin(),
        );
        assert!(
            allowed
                .authorizer
                .resolve_write(&claims_for("admin"), "task-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn assert_task_access_admin_bypasses_scope() {
        // A platform admin controls any user-scoped task, even one they don't own.
        let svc = service_with(
            tasks_returning(Some(task_fixture(Scope::User, Some("other-user")))),
            users_admin(),
        );
        assert!(
            svc.authorizer
                .resolve_write(&claims_for("admin"), "task-1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn assert_task_access_missing_task_is_not_found() {
        // Covers both a nonexistent task and one the scoped read filtered out —
        // an invisible task is indistinguishable from a missing one, like the
        // HTTP 404.
        let svc = service_with(tasks_returning(None), users_regular());
        let err = svc
            .authorizer
            .resolve_write(&claims_for("anyone"), "ghost")
            .await
            .expect_err("missing task must error");
        assert_eq!(err.code(), tonic::Code::NotFound);
    }

    /// Build a `TaskBound` verdict for the given owner + claimed task, as
    /// `verify_token` would produce from a task-bound JWT.
    fn task_bound(user_id: &str, claim_task_id: &str) -> VerifiedToken {
        let mut claims = claims_for(user_id);
        claims.task_id = Some(claim_task_id.to_string());
        VerifiedToken::TaskBound {
            claims,
            task_id: claim_task_id.to_string(),
        }
    }

    #[tokio::test]
    async fn task_bound_token_mismatch_is_denied() {
        // A token minted for task-1 must not act on task-2, even if its user
        // owns task-2 — the binding is the first, request-independent gate. The
        // control check never runs (no task load), so the repo returns nothing.
        let svc = service_with(tasks_returning(None), users_regular());
        let err = svc
            .authorizer
            .resolve_grant(task_bound("owner", "task-1"), "task-2")
            .await
            .expect_err("token bound to a different task must be denied");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn task_bound_token_match_allows_owner() {
        // Matching binding + owner control → granted, with the sandbox binding
        // echoed back on the grant.
        let grant = owner_task_at_generation(0)
            .resolve_grant(task_bound("owner", "task-1"), "task-1")
            .await
            .expect("matching binding + owner must be granted");
        assert_eq!(grant.sandbox_id, "task-1");
    }

    /// An enforcing authorizer over `owner`'s `task-1`, its own Sandbox at
    /// `generation`.
    fn owner_task_at_generation(generation: i64) -> SandboxAuthorizer {
        task_driven_by(Visibility::Private, generation, "owner")
    }

    /// An enforcing authorizer over `owner`'s `task-1` at `generation`, its
    /// tokens minted for `driver`.
    fn task_driven_by(visibility: Visibility, generation: i64, driver: &str) -> SandboxAuthorizer {
        let users: Arc<dyn CommitIdentity> = Arc::new(users_regular());
        let mut task = task_fixture(Scope::User, Some("owner"));
        task.visibility = visibility;
        SandboxAuthorizer::enforcing(
            "secret".to_string(),
            false,
            Some(tasks_returning(Some(task))),
            Some(users),
            Some(Arc::new(jwt::StaticTokenGenerations::one(
                "task-1",
                generation,
                &uid(driver).to_string(),
            ))),
        )
    }

    fn sandbox_bound(user_id: &str, generation: i64) -> VerifiedToken {
        let mut claims = claims_for(user_id);
        claims.sandbox_id = Some("task-1".to_string());
        claims.generation = generation;
        VerifiedToken::SandboxBound {
            claims,
            sandbox_id: "task-1".to_string(),
        }
    }

    #[tokio::test]
    async fn a_token_from_an_older_generation_is_refused() {
        let authorizer = owner_task_at_generation(2);

        let err = authorizer
            .resolve_grant(sandbox_bound("owner", 1), "task-1")
            .await
            .expect_err("a superseded token must be refused");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);

        let grant = authorizer
            .resolve_grant(sandbox_bound("owner", 2), "task-1")
            .await
            .expect("a current token is granted");
        assert_eq!(grant.token_generation, 2);
    }

    /// The done-when of drivers: once `visitor` drives `owner`'s public task,
    /// the owner's token at the same generation no longer reaches the pod.
    #[tokio::test]
    async fn only_the_driver_of_a_public_task_is_granted() {
        let authorizer = task_driven_by(Visibility::Public, 3, "visitor");

        let grant = authorizer
            .resolve_grant(sandbox_bound("visitor", 3), "task-1")
            .await
            .expect("the driver is granted");
        assert_eq!(grant.ctx.user_id, uid("visitor").to_string());

        for other in ["owner", "stranger"] {
            let err = authorizer
                .resolve_grant(sandbox_bound(other, 3), "task-1")
                .await
                .expect_err("anyone but the driver is refused");
            assert_eq!(err.code(), tonic::Code::Unauthenticated);
        }
    }
}
