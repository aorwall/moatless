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
    println!("API URL: {}", config.api_url);

    match &config.auth_mode {
        AuthMode::Sandbox { task_id, .. } => {
            println!("Mode:    sandbox");
            if let Some(ref tid) = task_id {
                println!("Task:    {tid}");
            }
            println!("Session: {}", config.session_file);
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
