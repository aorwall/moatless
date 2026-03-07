//! Info commands: whoami, status.

use crate::config::Config;

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
    println!("Backend: {}", config.backend_api_url);
    if let Some(ref task_id) = config.task_id {
        println!("Task:    {task_id}");
    } else {
        println!("Task:    (not in sandbox)");
    }
    println!("Session: {}", config.session_file);
}
