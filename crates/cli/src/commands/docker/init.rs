//! `moat docker init` — resolve work dir, seed compose assets, bootstrap
//! `.env`.

use std::fs;

use dialoguer::{Password, theme::ColorfulTheme};
use serde::{Deserialize, Serialize};

use super::InitArgs;
use super::docker_compose::{
    ComposeDirSource, ENV_EXAMPLE_FILE, ENV_FILE, INSTALL_ASSETS, resolve_compose_dir, seed_assets,
};
use super::env_file::{is_placeholder, read_env_map, set_env, write_env_file};
use crate::client::ClientError;
use crate::output;

/// Keys the installer manages. Other keys in `.env.example` get carried over
/// verbatim; these are the ones we prompt for or auto-generate.
const REQUIRED_KEYS: &[&str] = &[
    "CLAUDE_CODE_OAUTH_TOKEN",
    "TASK_TOKEN_SECRET",
    "SECRETS_ENCRYPTION_KEY",
];

#[derive(Serialize, Deserialize)]
pub(super) struct InitReport {
    pub(super) compose_dir: String,
    pub(super) compose_dir_source: String,
    pub(super) copied_assets: Vec<String>,
    pub(super) env_file: String,
    pub(super) env_created: bool,
    pub(super) keys_prompted: Vec<String>,
    pub(super) keys_generated: Vec<String>,
    pub(super) keys_missing: Vec<String>,
}

pub(super) async fn run(args: InitArgs, json: bool) -> Result<(), ClientError> {
    let (compose_dir, source) = resolve_compose_dir(args.compose_dir.as_deref())?;
    fs::create_dir_all(&compose_dir).map_err(|e| {
        ClientError::Other(format!("Failed to create {}: {e}", compose_dir.display()))
    })?;

    // 1. Ensure install assets are present. They're seeded from the binary's
    //    embedded copies (see INSTALL_ASSETS), so no external bundle is needed;
    //    `--assets-from` can override with an alternate bundle. Wrong-type
    //    entries are repaired in the loop below.
    let mut needs_seed = false;
    for (name, _) in INSTALL_ASSETS {
        let path = compose_dir.join(name);
        if !path.exists() {
            needs_seed = true;
        } else if !path.is_file() {
            // Remove a wrong-type entry (e.g. `docker compose` auto-created it
            // as a directory) so seeding writes a real file here. Deliberately
            // not forcing overwrite: that flag hits every entry and would
            // clobber the user's hand-edited assets, unrecoverable in this
            // cache dir.
            fs::remove_dir_all(&path).map_err(|e| {
                ClientError::Other(format!(
                    "Failed to remove stale {} (expected a file, found a directory): {e}",
                    path.display()
                ))
            })?;
            needs_seed = true;
        }
    }

    let mut copied_assets = Vec::new();
    if needs_seed {
        copied_assets = seed_assets(&compose_dir, args.assets_from.as_deref(), args.overwrite)?;
    } else if let (Some(assets), true) = (args.assets_from.as_deref(), args.overwrite) {
        copied_assets = seed_assets(&compose_dir, Some(assets), true)?;
    }

    // 2. Bootstrap `.env`. Merge existing `.env` over `.env.example` defaults.
    let env_path = compose_dir.join(ENV_FILE);
    let env_example_path = compose_dir.join(ENV_EXAMPLE_FILE);
    let env_created = !env_path.is_file();

    let example = read_env_map(&env_example_path)?;
    let mut current = if env_path.is_file() {
        read_env_map(&env_path)?
    } else {
        Vec::new()
    };

    for (k, v) in &example {
        if !current.iter().any(|(ek, _)| ek == k) {
            current.push((k.clone(), v.clone()));
        }
    }

    let mut keys_prompted = Vec::new();
    let mut keys_generated = Vec::new();
    let mut keys_missing = Vec::new();

    for key in REQUIRED_KEYS {
        let existing = current
            .iter()
            .find(|(k, _)| k == key)
            .map_or_default(|(_, v)| v.clone());
        if is_placeholder(key, &existing) {
            let new_value = match *key {
                "TASK_TOKEN_SECRET" => {
                    let v = crate::commands::generated_secret::hex_32();
                    keys_generated.push(key.to_string());
                    v
                }
                "SECRETS_ENCRYPTION_KEY" => {
                    // Backend expects base64 of 32 raw bytes (or 32 raw bytes).
                    let v = crate::commands::generated_secret::base64_32();
                    keys_generated.push(key.to_string());
                    v
                }
                "CLAUDE_CODE_OAUTH_TOKEN" => {
                    if args.non_interactive {
                        keys_missing.push(key.to_string());
                        continue;
                    }
                    let v = prompt_claude_token()?;
                    keys_prompted.push(key.to_string());
                    v
                }
                _ => unreachable!(),
            };
            set_env(&mut current, key, &new_value);
        }
    }

    write_env_file(&env_path, &current)?;

    let report = InitReport {
        compose_dir: compose_dir.display().to_string(),
        compose_dir_source: source.as_str().to_string(),
        copied_assets,
        env_file: env_path.display().to_string(),
        env_created,
        keys_prompted,
        keys_generated,
        keys_missing,
    };

    output::emit(json, &report, || print_human(&report, source));

    Ok(())
}

fn prompt_claude_token() -> Result<String, ClientError> {
    println!(
        "CLAUDE_CODE_OAUTH_TOKEN is required. Generate one with `claude setup-token` if you don't have it."
    );
    let token: String = Password::with_theme(&ColorfulTheme::default())
        .with_prompt("CLAUDE_CODE_OAUTH_TOKEN")
        .allow_empty_password(false)
        .interact()
        .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))?;
    Ok(token)
}

fn print_human(r: &InitReport, source: ComposeDirSource) {
    println!("Docker init");
    println!("===========");
    println!("Work dir:      {} ({})", r.compose_dir, source.as_str());
    if !r.copied_assets.is_empty() {
        println!("Copied assets: {}", r.copied_assets.join(", "));
    }
    println!(
        "Env file:      {} ({})",
        r.env_file,
        if r.env_created { "created" } else { "updated" }
    );
    if !r.keys_generated.is_empty() {
        println!("Generated:     {}", r.keys_generated.join(", "));
    }
    if !r.keys_prompted.is_empty() {
        println!("Set via prompt: {}", r.keys_prompted.join(", "));
    }
    if !r.keys_missing.is_empty() {
        println!("Missing keys:  {}", r.keys_missing.join(", "));
        for key in &r.keys_missing {
            println!("  → moat docker env set {key} <value>");
        }
    }
    println!();
    println!("Next: `moat docker install` to bring the stack up.");
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::super::InitArgs;
    use super::super::docker_compose::{COMPOSE_FILE, ENV_EXAMPLE_FILE};
    use super::run;

    /// Repairing one wrong-type asset must not clobber the user's hand-edited
    /// copies of the others.
    ///
    /// Fails against a repair branch that flips `overwrite` on for every entry
    /// instead of just the repaired one.
    #[tokio::test]
    async fn init_repairs_stale_dir_without_clobbering_user_edits() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("compose");
        fs::create_dir_all(&work).unwrap();

        // A user who hand-edited docker-compose.yml (e.g. remapped a port).
        let user_bytes = "services:\n  backend:\n    ports: [\"9999:8080\"]  # user edit\n";
        fs::write(work.join(COMPOSE_FILE), user_bytes).unwrap();

        // The wrong-type entry that triggers the repair path: a stale
        // .env.example *directory* instead of a file.
        fs::create_dir_all(work.join(ENV_EXAMPLE_FILE)).unwrap();

        let args = InitArgs {
            compose_dir: Some(work.clone()),
            assets_from: None,
            overwrite: false,
            non_interactive: true,
        };
        run(args, true).await.unwrap();

        assert!(
            work.join(ENV_EXAMPLE_FILE).is_file(),
            "stale .env.example directory should be repaired to a real file"
        );
        assert_eq!(
            fs::read_to_string(work.join(COMPOSE_FILE)).unwrap(),
            user_bytes,
            "user's hand-edited docker-compose.yml must not be clobbered by repair"
        );
    }
}
