//! Who a Task's commits are authored by, and who they credit as co-author.

use std::collections::HashMap;
use std::sync::Arc;

use moatless_ports::identity::CommitIdentity;
use moatless_types::credential_key::GIT_CO_AUTHOR;
use tracing::warn;

use crate::secrets::{GIT_USER_EMAIL, GIT_USER_NAME, TaskCredentialInputs};

/// Source the git author identity for the task's sandbox from the owning
/// `users` row, as `GIT_USER_NAME`/`GIT_USER_EMAIL` env vars.
///
/// Bot users get both fields from the row; human users get only the email
/// override (when set), leaving the name to GitHub-API resolution.
async fn fill_git_identity(
    credentials: &mut HashMap<String, String>,
    users: Option<&Arc<dyn CommitIdentity>>,
    user_id: &str,
) {
    let Some(users) = users else { return };
    // An actor that is not a user (the `system` sentinel, a `bot:` value) has
    // no git identity, and neither does one whose row is gone.
    let user = match users.git_identity(user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return,
        Err(e) => {
            warn!(error = %e, user_id, "Failed to look up user for git identity");
            return;
        }
    };
    if user.is_bot {
        credentials.insert(GIT_USER_NAME.to_string(), user.login);
        if let Some(email) = user.email {
            credentials.insert(GIT_USER_EMAIL.to_string(), email);
        }
        return;
    }
    // Human user: their Moatless account email when set, else the sandbox's
    // GitHub-API resolution at init. The name falls back to the login rather
    // than to that resolution: it is fixed at init, so after a driver switch
    // it would name the previous driver.
    let name = user
        .name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(user.login);
    credentials.insert(GIT_USER_NAME.to_string(), name);
    if let Some(email) = user.email {
        credentials.insert(GIT_USER_EMAIL.to_string(), email);
    }
}

/// Set the git author: the GitHub App's bot when the Task's pushes to
/// [`TaskCredentialInputs::github_repository`] go out as the App, else the
/// actor's own identity ([`fill_git_identity`]).
///
/// Returns the actor's user id when the actor authors, which
/// [`fill_co_author`] must not credit a second time.
pub(crate) async fn fill_commit_author(
    credentials: &mut HashMap<String, String>,
    input: &TaskCredentialInputs<'_>,
) -> Option<String> {
    if let (Some(issuer), Some(repo)) = (input.git_host_tokens, input.github_repository) {
        match issuer
            .github_app_commit_author(input.actor, repo.owner(), repo.name())
            .await
        {
            Ok(Some(bot)) => {
                credentials.insert(GIT_USER_NAME.to_string(), bot.login);
                if let Some(email) = bot.email {
                    credentials.insert(GIT_USER_EMAIL.to_string(), email);
                }
                return None;
            }
            Ok(None) => {}
            Err(error) => {
                warn!(%error, repository = %repo, "No App commit author, committing as the run-as identity");
            }
        }
    }
    let user_id = input.actor.user_id();
    fill_git_identity(credentials, input.users, user_id).await;
    Some(user_id.to_string())
}

/// Set [`GIT_CO_AUTHOR`] to `Name <email>` of the first human the
/// attribution credits who is not `author`. On GitHub the email is their
/// noreply address rather than their account email: GitHub credits a trailer
/// only to an email verified on the account.
pub(crate) async fn fill_co_author(
    credentials: &mut HashMap<String, String>,
    input: &TaskCredentialInputs<'_>,
    author: Option<String>,
) {
    let Some(users) = input.users else { return };
    for person in input.attribution.credited() {
        if author.as_deref() == Some(person) {
            continue;
        }
        let user = match users.git_identity(person).await {
            Ok(Some(u)) if !u.is_bot => u,
            Ok(_) => continue,
            Err(e) => {
                warn!(error = %e, person, "Failed to look up the co-author's git identity");
                continue;
            }
        };
        let noreply =
            match (input.git_host_tokens, input.github_repository) {
                (Some(issuer), Some(_)) => issuer
                    .github_noreply_email(person)
                    .await
                    .unwrap_or_else(|e| {
                        warn!(error = %e, person, "Failed to read the co-author's GitHub address");
                        None
                    }),
                _ => None,
            };
        let Some(email) = noreply.or(user.email) else {
            continue;
        };
        let name = user
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(user.login);
        // A newline or angle bracket would break out of the one trailer line
        // the sandbox writes this into.
        let unsafe_char = |c: char| c.is_control() || c == '<' || c == '>';
        if name.contains(unsafe_char) || email.contains(unsafe_char) || email.trim().is_empty() {
            continue;
        }
        credentials.insert(
            GIT_CO_AUTHOR.to_string(),
            format!("{} <{}>", name.trim(), email.trim()),
        );
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_ports::identity::{
        GitHostTokenIssuer, MockCommitIdentity, MockGitHostTokenIssuer,
    };
    use moatless_types::access_control::Actor;
    use moatless_workspace_types::repository::GitHostRepo;

    use crate::secrets::resolve_task_credentials;
    use moatless_types::attribution::Attribution;

    /// A `CommitIdentity` reporting one git author identity for every lookup.
    fn users_returning(
        login: &'static str,
        name: Option<&'static str>,
        email: Option<&'static str>,
        is_bot: bool,
    ) -> Arc<dyn CommitIdentity> {
        let mut users = MockCommitIdentity::new();
        users.expect_git_identity().returning(move |_| {
            Ok(Some(moatless_ports::identity::GitIdentity {
                is_bot,
                login: login.to_string(),
                name: name.map(str::to_string),
                email: email.map(str::to_string),
            }))
        });
        Arc::new(users)
    }

    fn people(driver: Option<&str>, reporter: Option<&str>, owner: Option<&str>) -> Attribution {
        Attribution {
            driver: driver.map(str::to_string),
            reporter: reporter.map(str::to_string),
            owner: owner.map(str::to_string),
        }
    }

    /// The git identity of the commits `actor` makes, as
    /// `(GIT_USER_NAME, GIT_USER_EMAIL, GIT_CO_AUTHOR)`.
    async fn commit_identity(
        actor: &Actor,
        attribution: &Attribution,
        users: &Arc<dyn CommitIdentity>,
        github: Option<(&Arc<dyn GitHostTokenIssuer>, &GitHostRepo)>,
    ) -> [Option<String>; 3] {
        let mut credentials = resolve_task_credentials(TaskCredentialInputs {
            task_id: "t1",
            actor,
            attribution,
            github_repository: github.map(|(_, repo)| repo),
            secrets: None,
            git_host_tokens: github.map(|(issuer, _)| issuer),
            users: Some(users),
            workspace_credentials: HashMap::new(),
            workspace_env_vars: HashMap::new(),
            gitlab_token_to_pods: false,
        })
        .await;
        [GIT_USER_NAME, GIT_USER_EMAIL, GIT_CO_AUTHOR].map(|key| credentials.remove(key))
    }

    async fn co_author_of(users: &Arc<dyn CommitIdentity>) -> Option<String> {
        let actor = Actor::of("run-as");
        let [.., co_author] =
            commit_identity(&actor, &people(Some("ada"), None, None), users, None).await;
        co_author
    }

    /// Every user is a human named and addressed by their id, except `bot`.
    fn users_by_id() -> Arc<dyn CommitIdentity> {
        let mut users = MockCommitIdentity::new();
        users.expect_git_identity().returning(|user| {
            Ok(Some(moatless_ports::identity::GitIdentity {
                is_bot: user == "bot",
                login: user.to_string(),
                name: Some(user.to_string()),
                email: Some(format!("{user}@example.com")),
            }))
        });
        Arc::new(users)
    }

    #[tokio::test]
    async fn the_person_behind_a_run_as_identity_is_the_co_author() {
        let named = users_returning("ada", Some("Ada Lovelace"), Some("ada@example.com"), false);
        assert_eq!(
            co_author_of(&named).await.as_deref(),
            Some("Ada Lovelace <ada@example.com>")
        );
        let unnamed = users_returning("ada", None, Some("ada@example.com"), false);
        assert_eq!(
            co_author_of(&unnamed).await.as_deref(),
            Some("ada <ada@example.com>"),
            "a person with no account name is credited by login"
        );
    }

    #[tokio::test]
    async fn the_reporter_is_credited_without_changing_the_execution_identity() {
        let users = users_by_id();
        let actor = Actor::of("bot").on_behalf_of(Some("loop-owner"));

        let identity = commit_identity(
            &actor,
            &people(None, Some("reporter"), Some("loop-owner")),
            &users,
            None,
        )
        .await;

        assert_eq!(
            identity,
            [
                Some("bot".to_string()),
                Some("bot@example.com".to_string()),
                Some("reporter <reporter@example.com>".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn a_person_who_drives_the_task_is_credited_over_its_reporter() {
        let users = users_by_id();
        let actor = Actor::of("bot").on_behalf_of(Some("driver"));

        let [.., co_author] = commit_identity(
            &actor,
            &people(Some("driver"), Some("reporter"), Some("loop-owner")),
            &users,
            None,
        )
        .await;

        assert_eq!(co_author.as_deref(), Some("driver <driver@example.com>"));
    }

    #[tokio::test]
    async fn a_person_driving_their_own_task_is_not_their_own_co_author() {
        let users = users_by_id();
        let identity = commit_identity(
            &Actor::of("ada"),
            &people(None, None, Some("ada")),
            &users,
            None,
        )
        .await;
        assert_eq!(
            identity,
            [
                Some("ada".to_string()),
                Some("ada@example.com".to_string()),
                None
            ]
        );
    }

    #[tokio::test]
    async fn the_app_authors_a_github_commit_and_credits_its_driver_by_noreply_address() {
        let users = users_by_id();
        let mut issuer = MockGitHostTokenIssuer::new();
        issuer
            .expect_github_app_commit_author()
            .withf(|actor, owner, repo| {
                actor.user_id() == "ada" && owner == "acme" && repo == "widgets"
            })
            .returning(|_, _, _| {
                Ok(Some(moatless_ports::identity::GitIdentity {
                    is_bot: true,
                    login: "moatless[bot]".to_string(),
                    name: Some("moatless[bot]".to_string()),
                    email: Some("99+moatless[bot]@users.noreply.github.com".to_string()),
                }))
            });
        issuer
            .expect_github_noreply_email()
            .returning(|user| Ok(Some(format!("1+{user}@users.noreply.github.com"))));
        let issuer: Arc<dyn GitHostTokenIssuer> = Arc::new(issuer);
        let repo = GitHostRepo::github("acme", "widgets");

        let identity = commit_identity(
            &Actor::of("ada"),
            &people(None, None, Some("ada")),
            &users,
            Some((&issuer, &repo)),
        )
        .await;

        assert_eq!(
            identity,
            [
                Some("moatless[bot]".to_string()),
                Some("99+moatless[bot]@users.noreply.github.com".to_string()),
                Some("ada <1+ada@users.noreply.github.com>".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn without_an_app_author_the_run_as_commits_and_its_account_email_credits() {
        let users = users_by_id();
        let mut issuer = MockGitHostTokenIssuer::new();
        issuer
            .expect_github_app_commit_author()
            .returning(|_, _, _| Ok(None));
        issuer.expect_github_noreply_email().returning(|_| Ok(None));
        let issuer: Arc<dyn GitHostTokenIssuer> = Arc::new(issuer);
        let repo = GitHostRepo::github("acme", "widgets");

        let identity = commit_identity(
            &Actor::of("bot").on_behalf_of(Some("driver")),
            &people(Some("driver"), None, None),
            &users,
            Some((&issuer, &repo)),
        )
        .await;

        assert_eq!(
            identity,
            [
                Some("bot".to_string()),
                Some("bot@example.com".to_string()),
                Some("driver <driver@example.com>".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn no_co_author_without_an_address_github_can_credit() {
        for users in [
            users_returning("ada", Some("Ada"), None, false),
            users_returning("bot", Some("Bot"), Some("bot@example.com"), true),
            users_returning(
                "ada",
                Some("Ada\nSigned-off-by: x"),
                Some("ada@example.com"),
                false,
            ),
            users_returning("ada", Some("Ada"), Some("ada@example.com> <x@y"), false),
        ] {
            assert_eq!(co_author_of(&users).await, None);
        }
    }

    #[tokio::test]
    async fn human_user_name_and_email_override() {
        // A regular (non-bot) user with both an account name and email should
        // have both flow through as GIT_USER_NAME / GIT_USER_EMAIL.
        let users = users_returning(
            "alice",
            Some("Alice Example"),
            Some("alice@example.com"),
            /* is_bot */ false,
        );

        let mut credentials = HashMap::new();
        fill_git_identity(&mut credentials, Some(&users), "u1").await;

        assert_eq!(
            credentials.get(GIT_USER_NAME).map(String::as_str),
            Some("Alice Example")
        );
        assert_eq!(
            credentials.get(GIT_USER_EMAIL).map(String::as_str),
            Some("alice@example.com")
        );
    }

    #[tokio::test]
    async fn human_user_without_a_name_commits_under_their_login() {
        let users = users_returning(
            "alice",
            None,
            Some("alice@example.com"),
            /* is_bot */ false,
        );

        let mut credentials = HashMap::new();
        fill_git_identity(&mut credentials, Some(&users), "u1").await;

        assert_eq!(
            credentials.get(GIT_USER_EMAIL).map(String::as_str),
            Some("alice@example.com")
        );
        assert_eq!(
            credentials.get(GIT_USER_NAME).map(String::as_str),
            Some("alice")
        );
    }

    #[tokio::test]
    async fn human_user_without_an_email_leaves_it_to_the_sandbox() {
        let users = users_returning("alice", None, None, /* is_bot */ false);

        let mut credentials = HashMap::new();
        fill_git_identity(&mut credentials, Some(&users), "u1").await;

        assert!(!credentials.contains_key(GIT_USER_EMAIL));
        assert_eq!(
            credentials.get(GIT_USER_NAME).map(String::as_str),
            Some("alice")
        );
    }

    #[tokio::test]
    async fn bot_user_sets_login_name_and_email() {
        // Bots keep the existing behavior: name is the login, plus email.
        let users = users_returning(
            "acme[bot]",
            None,
            Some("acme[bot]@users.noreply.github.com"),
            /* is_bot */ true,
        );

        let mut credentials = HashMap::new();
        fill_git_identity(&mut credentials, Some(&users), "u1").await;

        assert_eq!(
            credentials.get(GIT_USER_NAME).map(String::as_str),
            Some("acme[bot]")
        );
        assert_eq!(
            credentials.get(GIT_USER_EMAIL).map(String::as_str),
            Some("acme[bot]@users.noreply.github.com")
        );
    }
}
