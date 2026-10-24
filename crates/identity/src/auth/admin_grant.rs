//! Promote a sign-in identity listed in `ADMIN_USER_IDS` to admin.

use tracing::info;
use uuid::Uuid;

use super::error::AuthError;
use super::users_dao::UserDao;
use moatless_types::user::UserRole;

/// Sets `role = admin` on `user_id` when `admin_user_ids` lists
/// `{provider}|{subject}`.
///
/// It never demotes, so dropping an entry leaves a role granted in the UI
/// intact. `provider` is the stored identity key (`github`, `auth0`, or the
/// OIDC provider id) and `subject` the provider's user id.
pub(in crate::auth) async fn grant_listed_admin(
    users: &dyn UserDao,
    admin_user_ids: &[String],
    user_id: &Uuid,
    provider: &str,
    subject: &str,
) -> Result<(), AuthError> {
    let identity = format!("{provider}|{subject}");
    if !admin_user_ids.contains(&identity) {
        return Ok(());
    }
    let Some(user) = users.get_user(user_id).await? else {
        return Ok(());
    };
    if user.role == UserRole::Admin {
        return Ok(());
    }
    users
        .update_user_profile_and_role(user_id, None, None, Some(UserRole::Admin), None)
        .await?;
    info!(user_id = %user_id, login = %user.login, identity = %identity, "Granted admin from ADMIN_USER_IDS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::users::User;
    use crate::auth::users_dao::MockUserDao;
    use chrono::Utc;
    use moatless_types::user::UserStatus;

    fn user(id: Uuid, role: UserRole) -> User {
        User {
            id,
            login: "octocat".into(),
            email: Some("octocat@example.com".into()),
            name: None,
            avatar_url: None,
            provider: "github_app".into(),
            provider_user_id: Some("12345".into()),
            role,
            created_at: Utc::now(),
            last_login_at: Utc::now(),
            settings: serde_json::json!({}),
            is_bot: false,
            status: UserStatus::Active,
        }
    }

    fn listed() -> Vec<String> {
        vec!["github|12345".into()]
    }

    #[tokio::test]
    async fn listed_user_is_promoted() {
        let id = Uuid::new_v4();
        let mut users = MockUserDao::new();
        users
            .expect_get_user()
            .returning(move |_| Ok(Some(user(id, UserRole::User))));
        users
            .expect_update_user_profile_and_role()
            .withf(move |user_id, name, email, role, status| {
                *user_id == id
                    && name.is_none()
                    && email.is_none()
                    && *role == Some(UserRole::Admin)
                    && status.is_none()
            })
            .times(1)
            .returning(|_, _, _, _, _| Ok(None));

        grant_listed_admin(&users, &listed(), &id, "github", "12345")
            .await
            .expect("grant");
    }

    #[tokio::test]
    async fn unlisted_identity_is_left_alone() {
        // Covers demotion too: an admin absent from the list keeps its role.
        let mut users = MockUserDao::new();
        users.expect_get_user().never();
        users.expect_update_user_profile_and_role().never();

        grant_listed_admin(&users, &listed(), &Uuid::new_v4(), "auth0", "12345")
            .await
            .expect("no-op");
    }

    #[tokio::test]
    async fn existing_admin_is_not_rewritten() {
        let id = Uuid::new_v4();
        let mut users = MockUserDao::new();
        users
            .expect_get_user()
            .returning(move |_| Ok(Some(user(id, UserRole::Admin))));
        users.expect_update_user_profile_and_role().never();

        grant_listed_admin(&users, &listed(), &id, "github", "12345")
            .await
            .expect("no-op");
    }
}
