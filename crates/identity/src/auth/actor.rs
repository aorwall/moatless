//! Which identity acts.
//!
//! Every constructor below is one way an identity can be chosen. A reader who
//! opens this file can enumerate the rules without searching for them.
//!
//! Two rules cannot live here, because they read types this band cannot see:
//! the GitHub PR Adapter's outbound precedence (`TaskBinding`,
//! `SubscriptionMatch`) and its webhook sender mapping. Both stay in
//! `adapter-github-pr/src/auth.rs` and end in [`Actor::of`], which is why that
//! constructor exists.

use moatless_backend_core::scope::{ScopeContext, ScopedResource};

/// The identity a git-host call is made as.
///
/// There is no way to obtain a git-host token without naming one — see
/// [`crate::git_host_tokens::GitHostTokens`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Actor(String);

impl Actor {
    /// A user id chosen by a rule this band cannot express.
    pub fn of(user_id: impl Into<String>) -> Self {
        Self(user_id.into())
    }

    /// The caller of a request. An admin acts as themselves, never as the
    /// owner of what they are looking at.
    pub fn caller(ctx: &ScopeContext) -> Self {
        Self(ctx.user_id.clone())
    }

    /// The owner of a scoped resource — a Task, say.
    ///
    /// Reads `ScopedResource::owner_user_id`, which a Task fills from
    /// `owner_user_id` falling back to `created_by`, so this cannot drift from
    /// `Task::scoped_resource`.
    pub fn owner_of(resource: &ScopedResource) -> Option<Self> {
        resource
            .owner_user_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .map(Self::of)
    }

    /// The identity a Workspace runs as, falling back to the owner of the work.
    ///
    /// A Workspace names the account its Tasks act as, so it outranks whoever
    /// started the Task.
    pub fn run_as(workspace_run_as_user_id: Option<&str>, owner: Option<Self>) -> Option<Self> {
        workspace_run_as_user_id
            .filter(|id| !id.is_empty())
            .map(Self::of)
            .or(owner)
    }

    pub fn user_id(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Actor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_backend_core::scope::Scope;

    #[test]
    fn a_caller_acts_as_themselves_even_as_admin() {
        let ctx = ScopeContext {
            user_id: "admin".to_string(),
            is_admin: true,
        };
        assert_eq!(Actor::caller(&ctx).user_id(), "admin");
    }

    #[test]
    fn owner_of_falls_back_to_created_by_through_scoped_resource() {
        // `Task::scoped_resource` is `owner_user_id.or(created_by)`, so a row
        // predating `owner_user_id` still names an identity here.
        let backfilled = ScopedResource::borrowed(Scope::User, Some("creator"));
        assert_eq!(
            Actor::owner_of(&backfilled),
            Some(Actor::of("creator")),
            "a resource carrying only created_by must still name an actor"
        );

        let ownerless = ScopedResource::borrowed(Scope::Global, None);
        assert_eq!(Actor::owner_of(&ownerless), None);
    }

    #[test]
    fn a_workspace_run_as_identity_outranks_the_owner() {
        let owner = Some(Actor::of("starter"));
        assert_eq!(
            Actor::run_as(Some("bot"), owner.clone()),
            Some(Actor::of("bot"))
        );
        assert_eq!(Actor::run_as(None, owner.clone()), owner);
        assert_eq!(
            Actor::run_as(Some(""), owner.clone()),
            owner,
            "an empty column is not a named identity"
        );
        assert_eq!(Actor::run_as(None, None), None);
    }
}
