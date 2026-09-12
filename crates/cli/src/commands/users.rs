//! User directory and provider ID management.

use clap::Subcommand;
use moatless_api_schemas::auth::UserListResponse;
use moatless_api_schemas::enums::UserStatus;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum UserAction {
    /// List users — login, display name, email, bot, and status
    List {
        /// Include the disabled accounts, which are hidden by default
        #[arg(long)]
        include_disabled: bool,
    },
    /// Set a user's email address (admin-only when targeting another user)
    SetEmail {
        /// Moatless username (login), or the user id as `user list` reports it
        user: String,
        /// New email address
        email: String,
    },
    /// Set a user's display name (admin-only when targeting another user)
    SetName {
        /// Moatless username (login), or the user id as `user list` reports it
        user: String,
        /// New display name
        name: String,
    },
    /// Let a disabled account sign in and take work again (admin-only)
    Activate {
        /// Moatless username (login), or the user id as `user list` reports it
        user: String,
    },
    /// Disable an account — a departed person, or a superseded duplicate
    /// (admin-only)
    ///
    /// Every login path refuses a disabled account, its sessions and API keys
    /// stop working, and the directory hides it.
    ///
    /// A login shared by two accounts always names the older one. Pass the id
    /// to disable the newer.
    Deactivate {
        /// Moatless username (login), or the user id as `user list` reports it
        user: String,
    },
    /// Associate a provider-specific user ID with a moatless user
    SetProviderId {
        /// Moatless username (login)
        login: String,
        /// Provider name (e.g. "slack")
        #[arg(long)]
        provider: String,
        /// Provider-specific user ID (e.g. Slack user ID "U0000000002")
        #[arg(long)]
        id: String,
    },
    /// List all provider ID mappings for a user
    GetProviderIds {
        /// Moatless username (login)
        login: String,
    },
    /// Remove a provider ID mapping
    DeleteProviderId {
        /// Moatless username (login)
        login: String,
        /// Provider name (e.g. "slack")
        #[arg(long)]
        provider: String,
    },
}

pub async fn run(moat: &MoatClient, action: UserAction, json: bool) -> Result<(), ClientError> {
    match action {
        UserAction::List { include_disabled } => {
            let path = if include_disabled {
                "/api/v1/users?includeDisabled=true"
            } else {
                "/api/v1/users"
            };
            let resp = moat.raw_get(path).await?;
            let users = serde_json::from_value::<UserListResponse>(resp)
                .map_err(|e| ClientError::UndecodableResponse(e.to_string()))?
                .users;
            if json {
                output::print_json_typed(&UserListResponse { users });
            } else {
                let rows: Vec<Vec<String>> = users
                    .iter()
                    .map(|u| {
                        vec![
                            u.login.clone(),
                            u.name.clone().unwrap_or_else(|| "-".into()),
                            u.email.clone().unwrap_or_else(|| "-".into()),
                            u.is_bot.to_string(),
                            u.status.to_string(),
                        ]
                    })
                    .collect();
                output::print_table(&["LOGIN", "NAME", "EMAIL", "BOT", "STATUS"], &rows);
            }
        }

        UserAction::SetEmail { user, email } => {
            let body = serde_json::json!({ "email": email });
            let resp = patch_user(moat, &user, &body).await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                let updated = resp.get("email").and_then(|v| v.as_str()).unwrap_or(&email);
                println!("Updated {} email to {updated}", describe(&resp, &user));
            }
        }

        UserAction::SetName { user, name } => {
            let body = serde_json::json!({ "name": name });
            let resp = patch_user(moat, &user, &body).await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                let updated = resp.get("name").and_then(|v| v.as_str()).unwrap_or(&name);
                println!("Updated {} name to {updated}", describe(&resp, &user));
            }
        }

        UserAction::Activate { user } => {
            set_status(moat, &user, UserStatus::Active, json).await?;
        }

        UserAction::Deactivate { user } => {
            set_status(moat, &user, UserStatus::Disabled, json).await?;
        }

        UserAction::SetProviderId { login, provider, id } => {
            let body = serde_json::json!({ "provider_user_id": id });
            let resp = moat
                .raw_put(
                    &format!("/api/v1/users/{login}/provider-ids/{provider}"),
                    &body,
                )
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                println!("Provider ID for '{provider}' set successfully.");
            }
        }

        UserAction::GetProviderIds { login } => {
            let resp = moat
                .raw_get(&format!("/api/v1/users/{login}/provider-ids"))
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                let items = resp.as_array().cloned().unwrap_or_default();
                if items.is_empty() {
                    println!("No provider ID mappings found for '{login}'.");
                } else {
                    println!("{:<12} {:<38} CREATED AT", "PROVIDER", "PROVIDER USER ID");
                    for item in &items {
                        println!(
                            "{:<12} {:<38} {}",
                            item["provider"].as_str().unwrap_or(""),
                            item["provider_user_id"].as_str().unwrap_or(""),
                            item["created_at"].as_str().unwrap_or(""),
                        );
                    }
                }
            }
        }

        UserAction::DeleteProviderId { login, provider } => {
            let resp = moat
                .raw_delete(&format!("/api/v1/users/{login}/provider-ids/{provider}"))
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                println!("Provider ID for '{provider}' deleted.");
            }
        }
    }

    Ok(())
}

async fn set_status(
    moat: &MoatClient,
    user: &str,
    status: UserStatus,
    json: bool,
) -> Result<(), ClientError> {
    let body = serde_json::json!({ "status": status });
    let resp = patch_user(moat, user, &body).await?;
    if json {
        output::print_json_typed(&resp);
    } else {
        println!("{} is now {status}.", describe(&resp, user));
    }
    Ok(())
}

/// PATCH the user `target` names, by id when it parses as a uuid.
///
/// A login is tried only when the target is not a uuid, so a login shaped like
/// one still resolves to the row it names. The id-keyed path is the only way to
/// reach the newer account of a login two accounts share: the login-keyed path
/// answers with the older one.
async fn patch_user(
    moat: &MoatClient,
    target: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, ClientError> {
    let path = match uuid::Uuid::parse_str(target) {
        Ok(id) => format!("/api/v1/admin/users/{id}"),
        Err(_) => format!("/api/v1/users/{target}"),
    };
    moat.raw_patch(&path, body).await
}

/// Name the account a PATCH landed on, as `'login' (id)`.
///
/// A login alone is ambiguous in exactly the case the id-keyed path exists for.
fn describe(resp: &serde_json::Value, target: &str) -> String {
    let login = resp.get("login").and_then(|v| v.as_str()).unwrap_or(target);
    match resp.get("id").and_then(|v| v.as_str()) {
        Some(id) => format!("'{login}' ({id})"),
        None => format!("'{login}'"),
    }
}
