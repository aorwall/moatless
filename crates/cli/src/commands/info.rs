//! Info commands: whoami, status.

use moatless_api_schemas::auth::SessionResponse;

use crate::config::{AuthMode, Config};

pub fn format_whoami(resp: &SessionResponse) {
    if let Some(ref user) = resp.user {
        println!("User:  {}", user.login);
        println!("Name:  {}", user.name.as_deref().unwrap_or("(not set)"));
        println!("Email: {}", user.email.as_deref().unwrap_or("(not set)"));
        println!("ID:    {}", user.id);
        println!("Role:  {}", user.role);
    } else {
        println!("Not authenticated.");
    }
}

pub fn print_status(config: &Config) {
    println!(
        "API URL: {}",
        config.api_url.as_deref().unwrap_or("(not set)")
    );

    match &config.auth_mode {
        AuthMode::Sandbox { task_id, .. } => {
            println!("Mode:    sandbox");
            if let Some(tid) = task_id {
                println!("Task:    {tid}");
            }
        }
        AuthMode::Standalone { api_key } => {
            println!("Mode:    standalone");
            let host = config.host();
            println!("Profile: {host}");
            let creds = crate::credentials::load_credentials();
            let profile = creds.profiles.get(&host);
            if api_key.is_some() {
                if let Some(p) = profile {
                    println!("User:    {}", p.user_login);
                } else {
                    println!("Auth:    API key set");
                }
            } else if let Some(p) = profile {
                // Profile exists but api_key is empty — registered via
                // `moat login --local` for a no-auth install.
                println!("User:    {} (no auth)", p.user_login);
            } else {
                println!("Auth:    not logged in (run `moat login`)");
            }
        }
    }
}

pub async fn whoami(
    moat: &crate::client::MoatClient,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let session = moat.api().me().await?;
    crate::output::emit(json, &session, || format_whoami(&session));
    Ok(())
}

pub async fn tags(
    moat: &crate::client::MoatClient,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let tags = moat.api().list_tags().await?;
    crate::output::emit(json, &tags, || {
        let rows: Vec<Vec<String>> = tags
            .iter()
            .map(|t| vec![t.tag_id.clone(), t.name.clone()])
            .collect();
        crate::output::print_table(&["TAG_ID", "NAME"], &rows);
    });
    Ok(())
}
