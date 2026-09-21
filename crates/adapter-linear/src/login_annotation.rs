//! Naming a Linear user in Moatless terms on the way out to the agent.
//!
//! [`reference_resolution`](super::reference_resolution) runs the other
//! direction, turning a reference the agent wrote into a Linear user. Here a
//! Linear user object gains [`LOGIN_FIELD`], so the agent reading a response
//! has the one name it can address someone by. Nothing Linear sent is removed:
//! a caller that wants the real `displayName` still has it.

use serde_json::Value;
use tracing::warn;

use moatless_ports::identity::ProviderIdentityLink;

use super::reference_resolution::LINEAR_PROVIDER;

/// The Moatless login attached to a Linear user object. camelCase to sit
/// beside Linear's own fields, and a name Linear's schema does not use, so it
/// can only ever be ours.
pub(super) const LOGIN_FIELD: &str = "moatlessLogin";

/// Attach [`LOGIN_FIELD`] to every user an issue node carries.
pub(super) async fn annotate_issue(identity: &dyn ProviderIdentityLink, issue: &mut Value) {
    annotate(identity, issue, visit_issue_users).await;
}

/// [`annotate_issue`] over a list, resolving each distinct user once across the
/// whole page rather than once per issue.
pub(super) async fn annotate_issues(identity: &dyn ProviderIdentityLink, issues: &mut Vec<Value>) {
    annotate(identity, issues, |issues, f| {
        for issue in issues.iter_mut() {
            visit_issue_users(issue, f);
        }
    })
    .await;
}

/// Attach [`LOGIN_FIELD`] to a node that is itself a user, as `lookup user`
/// returns.
pub(super) async fn annotate_user(identity: &dyn ProviderIdentityLink, user: &mut Value) {
    annotate(identity, user, |user, f| {
        if user.is_object() {
            f(user);
        }
    })
    .await;
}

/// Resolve every user `visit` reaches, then write the logins back.
///
/// Two passes over the tree rather than one: the lookups are async and the
/// visitor hands out `&mut`, which cannot be held across an await. Resolving
/// between them also means a user in ten rows costs one directory read.
async fn annotate<T: ?Sized>(
    identity: &dyn ProviderIdentityLink,
    root: &mut T,
    visit: impl Fn(&mut T, &mut dyn FnMut(&mut Value)),
) {
    let mut ids: Vec<String> = Vec::new();
    visit(root, &mut |user| {
        if let Some(id) = linear_id(user) {
            ids.push(id.to_string());
        }
    });
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return;
    }

    let mut logins: Vec<(String, String)> = Vec::new();
    for id in ids {
        match identity.user_at(LINEAR_PROVIDER, &id).await {
            Ok(Some(user)) => logins.push((id, user.login)),
            // Nobody has linked this Linear account. The agent sees no
            // `moatlessLogin`, which is the honest answer.
            Ok(None) => {}
            Err(e) => warn!(
                error = %e,
                linear_user = %id,
                "Linear user left without a Moatless login: the directory lookup failed"
            ),
        }
    }

    visit(root, &mut |user| {
        let Some(id) = linear_id(user) else { return };
        let Some((_, login)) = logins.iter().find(|(known, _)| known == id) else {
            return;
        };
        let login = Value::String(login.clone());
        if let Some(object) = user.as_object_mut() {
            object.insert(LOGIN_FIELD.to_string(), login);
        }
    });
}

/// The Linear user id a user object is keyed by, which is what
/// `user_provider_ids` stores against a Moatless account.
fn linear_id(user: &Value) -> Option<&str> {
    user.get("id")?.as_str().filter(|id| !id.is_empty())
}

/// The users an issue node carries: its three role fields, and the author of
/// every comment in the `comments` connection `get_issue` selects.
fn visit_issue_users(issue: &mut Value, f: &mut dyn FnMut(&mut Value)) {
    let Some(issue) = issue.as_object_mut() else {
        return;
    };
    for role in ["assignee", "delegate", "creator"] {
        if let Some(user) = issue.get_mut(role).filter(|user| user.is_object()) {
            f(user);
        }
    }
    if let Some(nodes) = issue
        .get_mut("comments")
        .and_then(|comments| comments.get_mut("nodes"))
        .and_then(Value::as_array_mut)
    {
        for node in nodes {
            if let Some(user) = node.get_mut("user").filter(|user| user.is_object()) {
                f(user);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use moatless_ports::identity::{LinkedUser, ProviderIdentityLink};
    use moatless_types::store::StoreError;
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uuid::Uuid;

    use super::{annotate_issue, annotate_issues, annotate_user};

    /// Knows one link: Linear `u-martin` is the Moatless login `martsam-fp`.
    /// Counts its reads so a test can assert the dedupe.
    struct Directory {
        reads: AtomicUsize,
        fail: bool,
    }

    impl Directory {
        fn new() -> Self {
            Self {
                reads: AtomicUsize::new(0),
                fail: false,
            }
        }
        fn failing() -> Self {
            Self {
                reads: AtomicUsize::new(0),
                fail: true,
            }
        }
        fn reads(&self) -> usize {
            self.reads.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl ProviderIdentityLink for Directory {
        async fn user_at(
            &self,
            provider: &str,
            provider_user_id: &str,
        ) -> Result<Option<LinkedUser>, StoreError> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            if self.fail {
                return Err(StoreError::new("the directory is down"));
            }
            Ok(
                (provider == "linear" && provider_user_id == "u-martin").then(|| LinkedUser {
                    id: Uuid::nil(),
                    login: "martsam-fp".to_string(),
                }),
            )
        }
        async fn user_by_email(&self, _: &str) -> Result<Option<LinkedUser>, StoreError> {
            Ok(None)
        }
        async fn provider_id_for_login(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<String>, StoreError> {
            Ok(None)
        }
        async fn user_who_connected(&self, _: &str, _: &str) -> Result<Option<String>, StoreError> {
            Ok(None)
        }
        async fn user_with_login(&self, _: &str, _: &str) -> Result<Option<String>, StoreError> {
            Ok(None)
        }
        async fn link(&self, _: &Uuid, _: &str, _: &str) -> Result<bool, StoreError> {
            Ok(false)
        }
    }

    fn martin() -> Value {
        json!({
            "id": "u-martin",
            "name": "Martin Samuelsson",
            "displayName": "martin.samuelsson",
            "email": "martin.samuelsson@soaplabs.com"
        })
    }

    fn stranger() -> Value {
        json!({ "id": "u-stranger", "displayName": "someone.else" })
    }

    /// The incident this exists to prevent: the agent read `displayName` off an
    /// assignee and mentioned it, because nothing in the response named the
    /// login it could actually address.
    #[tokio::test]
    async fn an_assignee_gains_the_login_without_losing_linears_own_fields() {
        let mut issue = json!({ "identifier": "SP-3017", "assignee": martin() });
        annotate_issue(&Directory::new(), &mut issue).await;
        assert_eq!(issue["assignee"]["moatlessLogin"], json!("martsam-fp"));
        assert_eq!(
            issue["assignee"]["displayName"],
            json!("martin.samuelsson"),
            "Linear's own name survives: this annotates, it does not rewrite"
        );
        assert_eq!(issue["identifier"], json!("SP-3017"));
    }

    #[tokio::test]
    async fn every_role_and_every_comment_author_is_reached() {
        let mut issue = json!({
            "assignee": martin(),
            "delegate": martin(),
            "creator": stranger(),
            "comments": { "nodes": [
                { "body": "one", "user": martin() },
                { "body": "two", "user": stranger() },
            ]},
        });
        annotate_issue(&Directory::new(), &mut issue).await;
        for named in [&issue["assignee"], &issue["delegate"]] {
            assert_eq!(named["moatlessLogin"], json!("martsam-fp"));
        }
        assert_eq!(
            issue["comments"]["nodes"][0]["user"]["moatlessLogin"],
            json!("martsam-fp")
        );
        for unlinked in [&issue["creator"], &issue["comments"]["nodes"][1]["user"]] {
            assert!(
                unlinked.get("moatlessLogin").is_none(),
                "nobody is linked to this Linear account, so there is no login to claim"
            );
        }
    }

    /// One directory read per distinct user, not one per slot: a `list` of 50
    /// issues assigned to the same person would otherwise pay 50 times.
    #[tokio::test]
    async fn a_user_in_many_slots_costs_one_lookup() {
        let directory = Directory::new();
        let mut issues = vec![
            json!({ "assignee": martin(), "creator": martin() }),
            json!({ "assignee": martin(), "delegate": stranger() }),
        ];
        annotate_issues(&directory, &mut issues).await;
        assert_eq!(
            directory.reads(),
            2,
            "`u-martin` and `u-stranger`, once each"
        );
        assert_eq!(issues[1]["assignee"]["moatlessLogin"], json!("martsam-fp"));
    }

    #[tokio::test]
    async fn a_lookup_result_is_annotated_in_place() {
        let mut user = martin();
        annotate_user(&Directory::new(), &mut user).await;
        assert_eq!(user["moatlessLogin"], json!("martsam-fp"));
    }

    /// Naming someone is never worth failing the command over: the agent still
    /// gets the issue, just without the login.
    #[tokio::test]
    async fn a_directory_failure_leaves_the_payload_intact() {
        let mut issue = json!({ "identifier": "SP-3017", "assignee": martin() });
        annotate_issue(&Directory::failing(), &mut issue).await;
        assert!(issue["assignee"].get("moatlessLogin").is_none());
        assert_eq!(issue["assignee"]["displayName"], json!("martin.samuelsson"));
    }

    #[tokio::test]
    async fn an_issue_with_nobody_on_it_reads_the_directory_not_at_all() {
        let directory = Directory::new();
        let mut issue = json!({ "identifier": "SP-1", "assignee": Value::Null });
        annotate_issue(&directory, &mut issue).await;
        assert_eq!(directory.reads(), 0);
        assert_eq!(issue["assignee"], Value::Null);
    }
}
