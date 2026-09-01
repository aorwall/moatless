//! `moat docker env` — inspect and update keys in `<compose-dir>/.env`.
//!
//! Exists so callers (Claude, scripts) can set secret values without editing
//! the file directly. `env set CLAUDE_CODE_OAUTH_TOKEN <token>` writes the
//! value and logs a redacted summary; `env list` shows which keys are set,
//! placeholder, or empty without leaking secret values.

use std::fs;

use serde::{Deserialize, Serialize};

use super::env_file::{is_placeholder, is_secret, read_env_map, set_env, write_env_file};
use super::compose::{resolve_compose_dir, ComposeDirSource, ENV_EXAMPLE_FILE, ENV_FILE};
use super::{EnvAction, EnvListArgs, EnvSetArgs, EnvUnsetArgs};
use crate::client::ClientError;
use crate::output;

#[derive(Serialize, Deserialize)]
pub(super) struct EnvSetReport {
    pub(super) compose_dir: String,
    pub(super) env_file: String,
    pub(super) key: String,
    pub(super) action: String, // "created" | "updated" | "unchanged"
    pub(super) was_placeholder: bool,
    pub(super) is_secret: bool,
}

#[derive(Serialize, Deserialize)]
pub(super) struct EnvUnsetReport {
    pub(super) compose_dir: String,
    pub(super) env_file: String,
    pub(super) key: String,
    pub(super) action: String, // "removed" | "not_present"
}

#[derive(Serialize, Deserialize)]
pub(super) struct EnvListReport {
    pub(super) compose_dir: String,
    pub(super) env_file: String,
    pub(super) env_present: bool,
    pub(super) keys: Vec<EnvKey>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct EnvKey {
    pub(super) name: String,
    pub(super) status: String, // "set" | "placeholder" | "empty"
    pub(super) is_secret: bool,
    /// Masked for secrets, full value otherwise.
    pub(super) display: String,
}

pub(super) async fn run(action: EnvAction, json: bool) -> Result<(), ClientError> {
    match action {
        EnvAction::Set(args) => run_set(args, json).await,
        EnvAction::Unset(args) => run_unset(args, json).await,
        EnvAction::List(args) => run_list(args, json).await,
    }
}

async fn run_unset(args: EnvUnsetArgs, json: bool) -> Result<(), ClientError> {
    let (compose_dir, _source) = resolve_compose_dir(args.compose_dir.as_deref())?;
    let env_path = compose_dir.join(ENV_FILE);

    if !env_path.is_file() {
        return Err(ClientError::Other(format!(
            "{} not found. Nothing to unset.",
            env_path.display()
        )));
    }

    let mut env = read_env_map(&env_path)?;
    let before = env.len();
    env.retain(|(k, _)| k != &args.key);
    let action = if env.len() < before {
        write_env_file(&env_path, &env)?;
        "removed"
    } else {
        "not_present"
    };

    let report = EnvUnsetReport {
        compose_dir: compose_dir.display().to_string(),
        env_file: env_path.display().to_string(),
        key: args.key.clone(),
        action: action.to_string(),
    };

    if json {
        output::print_json_typed(&report);
    } else {
        print_unset_human(&report);
    }
    Ok(())
}

async fn run_set(args: EnvSetArgs, json: bool) -> Result<(), ClientError> {
    let (compose_dir, _source) = resolve_compose_dir(args.compose_dir.as_deref())?;
    let env_path = compose_dir.join(ENV_FILE);
    let example_path = compose_dir.join(ENV_EXAMPLE_FILE);

    // Seed `.env` from `.env.example` if it's missing but the example exists.
    // Matches `init`'s behavior so `env set` works as a first step too.
    let created = if env_path.is_file() {
        false
    } else if example_path.is_file() {
        fs::copy(&example_path, &env_path).map_err(|e| {
            ClientError::Other(format!("Failed to seed {}: {e}", env_path.display()))
        })?;
        true
    } else {
        return Err(ClientError::Other(format!(
            "{} not found. Run `moat docker init` first to seed it.",
            env_path.display()
        )));
    };

    let mut env = read_env_map(&env_path)?;
    let previous = env
        .iter()
        .find(|(k, _)| k == &args.key)
        .map(|(_, v)| v.clone());

    let was_placeholder = previous
        .as_deref()
        .is_some_and(|v| is_placeholder(&args.key, v));

    let action = match previous.as_deref() {
        None => "created",
        Some(existing) if existing == args.value => "unchanged",
        Some(_) => "updated",
    };

    if action != "unchanged" {
        set_env(&mut env, &args.key, &args.value);
        write_env_file(&env_path, &env)?;
    }

    let report = EnvSetReport {
        compose_dir: compose_dir.display().to_string(),
        env_file: env_path.display().to_string(),
        key: args.key.clone(),
        action: if created {
            // If we seeded the file in this call, the key state is also "created".
            "created".to_string()
        } else {
            action.to_string()
        },
        was_placeholder,
        is_secret: is_secret(&args.key),
    };

    if json {
        output::print_json_typed(&report);
    } else {
        print_set_human(&report);
    }
    Ok(())
}

async fn run_list(args: EnvListArgs, json: bool) -> Result<(), ClientError> {
    let (compose_dir, source) = resolve_compose_dir(args.compose_dir.as_deref())?;
    let env_path = compose_dir.join(ENV_FILE);
    let env_present = env_path.is_file();
    let env = if env_present {
        read_env_map(&env_path)?
    } else {
        Vec::new()
    };

    let keys: Vec<EnvKey> = env
        .iter()
        .map(|(k, v)| {
            let secret = is_secret(k);
            let status = if v.is_empty() {
                "empty"
            } else if is_placeholder(k, v) {
                "placeholder"
            } else {
                "set"
            };
            let display = if status != "set" {
                v.clone()
            } else if secret {
                mask_secret(v)
            } else {
                v.clone()
            };
            EnvKey {
                name: k.clone(),
                status: status.to_string(),
                is_secret: secret,
                display,
            }
        })
        .collect();

    let report = EnvListReport {
        compose_dir: compose_dir.display().to_string(),
        env_file: env_path.display().to_string(),
        env_present,
        keys,
    };

    if json {
        output::print_json_typed(&report);
    } else {
        print_list_human(&report, source);
    }
    Ok(())
}

/// Secret values render as `****` plus the length (no prefix/suffix leak).
fn mask_secret(v: &str) -> String {
    format!("**** ({} chars)", v.chars().count())
}

fn print_set_human(r: &EnvSetReport) {
    println!("Docker env set");
    println!("==============");
    println!("Env file: {}", r.env_file);
    println!("Key:      {}", r.key);
    println!("Action:   {}", r.action);
    if r.was_placeholder {
        println!("Note:     replaced the `your_*_here` placeholder");
    }
    if r.is_secret {
        println!("Value:    (secret, not echoed)");
    }
}

fn print_unset_human(r: &EnvUnsetReport) {
    println!("Docker env unset");
    println!("================");
    println!("Env file: {}", r.env_file);
    println!("Key:      {}", r.key);
    println!("Action:   {}", r.action);
}

fn print_list_human(r: &EnvListReport, source: ComposeDirSource) {
    println!("Docker env list");
    println!("===============");
    println!("Work dir: {} ({})", r.compose_dir, source.as_str());
    println!("Env file: {}", r.env_file);
    if !r.env_present {
        println!();
        println!("`.env` not found — run `moat docker init` to seed it.");
        return;
    }
    println!();

    let headers = ["KEY", "STATUS", "VALUE"];
    let rows: Vec<Vec<String>> = r
        .keys
        .iter()
        .map(|k| vec![k.name.clone(), k.status.clone(), k.display.clone()])
        .collect();
    output::print_table(&headers, &rows);
}
