//! Info commands: whoami, status.

use crate::config::{AuthMode, Config};

pub fn format_whoami(resp: &serde_json::Value) {
    if let Some(user) = resp.get("user") {
        let id = user.get("id").and_then(|v| v.as_str()).unwrap_or("-");
        let login = user.get("login").and_then(|v| v.as_str()).unwrap_or("-");
        let name = user
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("(not set)");
        let email = user
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("(not set)");
        let role = user.get("role").and_then(|v| v.as_str()).unwrap_or("-");

        println!("User:  {login}");
        println!("Name:  {name}");
        println!("Email: {email}");
        println!("ID:    {id}");
        println!("Role:  {role}");
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
            if api_key.is_some() {
                let creds = crate::credentials::load_credentials();
                if let Some(profile) = creds.profiles.get(&host) {
                    println!("User:    {}", profile.user_login);
                } else {
                    println!("Auth:    API key set");
                }
            } else {
                println!("Auth:    not logged in (run `moat login`)");
            }
        }
    }
}
