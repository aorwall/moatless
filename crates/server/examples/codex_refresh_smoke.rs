//! Local smoke test for `refresh_chatgpt_tokens`.
//!
//! Reads a Codex `auth.json` (default `$HOME/.codex/auth.json`, override via
//! `CODEX_AUTH_JSON_PATH`) and calls the live refresh endpoint (mock via
//! `CODEX_REFRESH_TOKEN_URL_OVERRIDE`).
//!
//! **WARNING**: refresh rotates `refresh_token` server-side, killing the old
//! one — against SoapLabBot's blob, persist the rotated value immediately, or
//! set `CODEX_WRITE_BACK=1` to write it back to the source file (off by
//! default).

#![allow(clippy::print_stdout)] // reason: smoke example writes to stdout by design

use std::error::Error;

use moatless_identity::credentials::codex::oauth::RefreshedTokens;
use moatless_identity::credentials::codex::oauth::parse_jwt_exp;
use moatless_identity::credentials::codex::oauth::refresh_chatgpt_tokens;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::var("CODEX_AUTH_JSON_PATH").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        format!("{home}/.codex/auth.json")
    });

    println!("Reading Codex auth.json from {path}");
    let body = tokio::fs::read_to_string(&path).await?;
    let mut blob: serde_json::Value = serde_json::from_str(&body)?;

    if let Some(at) = blob
        .get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(serde_json::Value::as_str)
    {
        if let Ok(Some(exp)) = parse_jwt_exp(at) {
            println!("old access_token exp: {exp}");
        }
    }

    let refresh_token = blob
        .get("tokens")
        .and_then(|t| t.get("refresh_token"))
        .and_then(serde_json::Value::as_str)
        .ok_or("no tokens.refresh_token in auth.json")?
        .to_string();

    let client = reqwest::Client::new();
    println!("Calling refresh_chatgpt_tokens (this rotates the refresh_token!)...");
    let refresh_url = moatless_core::config::codex_refresh_url_from_env();
    let new = refresh_chatgpt_tokens(&client, &refresh_url, &refresh_token).await?;

    println!("id_token:      {}", new.id_token.is_some());
    println!("access_token:  {}", new.access_token.is_some());
    println!("refresh_token: {}", new.refresh_token.is_some());

    if let Some(at) = new.access_token.as_deref() {
        match parse_jwt_exp(at) {
            Ok(Some(exp)) => println!("new access_token exp: {exp}"),
            Ok(None) => println!("new access_token has no exp claim"),
            Err(e) => println!("could not decode new access_token: {e}"),
        }
    }

    if std::env::var("CODEX_WRITE_BACK").is_ok() {
        merge_rotated_into_blob(&mut blob, &new);
        let serialized = serde_json::to_string_pretty(&blob)?;
        tokio::fs::write(&path, serialized).await?;
        println!("Wrote rotated blob back to {path}");
    } else {
        println!(
            "NOT writing back. Source file's refresh_token is now DEAD upstream — \
             rerun with CODEX_WRITE_BACK=1 next time to keep the file usable."
        );
    }

    Ok(())
}

/// Mirror of `credentials::merge_refreshed_blob` — only overwrites the fields
/// the server actually rotated, and updates `last_refresh`.
fn merge_rotated_into_blob(existing: &mut serde_json::Value, new: &RefreshedTokens) {
    if let Some(tokens) = existing
        .get_mut("tokens")
        .and_then(serde_json::Value::as_object_mut)
    {
        if let Some(ref id_token) = new.id_token {
            tokens.insert(
                "id_token".to_string(),
                serde_json::Value::String(id_token.clone()),
            );
        }
        if let Some(ref access_token) = new.access_token {
            tokens.insert(
                "access_token".to_string(),
                serde_json::Value::String(access_token.clone()),
            );
        }
        if let Some(ref refresh_token) = new.refresh_token {
            tokens.insert(
                "refresh_token".to_string(),
                serde_json::Value::String(refresh_token.clone()),
            );
        }
    }
    if let Some(obj) = existing.as_object_mut() {
        obj.insert(
            "last_refresh".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
    }
}
