//! Persistent credential storage in `~/.config/moatless/credentials.json`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct CredentialsFile {
    pub default: Option<String>,
    pub profiles: HashMap<String, Profile>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Profile {
    pub api_key: String,
    pub user_login: String,
    pub created_at: String,
}

/// Path to the credentials file: `~/.config/moatless/credentials.json`.
fn credentials_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("moatless").join("credentials.json"))
}

/// Load the credentials file, returning default if missing or invalid.
pub fn load_credentials() -> CredentialsFile {
    let Some(path) = credentials_path() else {
        return CredentialsFile::default();
    };
    let Ok(data) = fs::read_to_string(&path) else {
        return CredentialsFile::default();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

/// Save credentials to disk with restricted permissions.
pub fn save_credential(host: &str, api_key: &str, login: &str) -> Result<(), String> {
    let path = credentials_path().ok_or("Cannot determine config directory")?;
    let dir = path.parent().ok_or("Invalid credentials path")?;
    fs::create_dir_all(dir).map_err(|e| format!("Failed to create config dir: {e}"))?;

    let mut creds = load_credentials();
    creds.profiles.insert(
        host.to_string(),
        Profile {
            api_key: api_key.to_string(),
            user_login: login.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    );
    if creds.default.is_none() {
        creds.default = Some(host.to_string());
    }

    let json = serde_json::to_string_pretty(&creds)
        .map_err(|e| format!("Failed to serialize credentials: {e}"))?;
    fs::write(&path, &json).map_err(|e| format!("Failed to write credentials: {e}"))?;

    // Set file permissions to 0o600 (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&path, perms).map_err(|e| format!("Failed to set permissions: {e}"))?;
    }

    Ok(())
}

/// Delete credential for a given host.
pub fn delete_credential(host: &str) -> Result<bool, String> {
    let path = credentials_path().ok_or("Cannot determine config directory")?;
    if !path.exists() {
        return Ok(false);
    }

    let mut creds = load_credentials();
    let removed = creds.profiles.remove(host).is_some();
    if removed {
        // Clear default if it pointed to the removed host
        if creds.default.as_deref() == Some(host) {
            creds.default = creds.profiles.keys().next().cloned();
        }
        let json = serde_json::to_string_pretty(&creds)
            .map_err(|e| format!("Failed to serialize credentials: {e}"))?;
        fs::write(&path, &json).map_err(|e| format!("Failed to write credentials: {e}"))?;
    }
    Ok(removed)
}

/// Get the API key for a given host from the credentials file.
pub fn get_api_key(host: &str) -> Option<String> {
    let creds = load_credentials();
    creds.profiles.get(host).map(|p| p.api_key.clone())
}
