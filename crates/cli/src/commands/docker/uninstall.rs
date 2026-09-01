//! `moat docker uninstall` — `docker compose down` + optional cleanup.

use std::fs;

use dialoguer::{theme::ColorfulTheme, Confirm};
use serde::{Deserialize, Serialize};

use super::compose::{
    cache_compose_dir, detect_compose_cmd, resolve_compose_dir, run_compose_streaming,
    ComposeDirSource,
};
use super::UninstallArgs;
use crate::client::ClientError;
use crate::output;

#[derive(Serialize, Deserialize)]
pub(super) struct UninstallReport {
    pub(super) compose_dir: String,
    pub(super) compose_dir_source: String,
    pub(super) volumes_purged: bool,
    pub(super) cache_purged: bool,
}

pub(super) async fn run(args: UninstallArgs, json: bool) -> Result<(), ClientError> {
    let (compose_dir, source) = resolve_compose_dir(args.compose_dir.as_deref())?;
    let compose = detect_compose_cmd().await?;

    if args.purge && std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("--purge drops postgres + redis volumes. Continue?")
            .default(false)
            .interact()
            .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))?;
        if !confirmed {
            return Err(ClientError::Other("Aborted by user.".to_string()));
        }
    }

    let mut down_args = vec!["down"];
    if args.purge {
        down_args.push("-v");
    }
    run_compose_streaming(&compose, &compose_dir, &down_args).await?;

    let mut cache_purged = false;
    if args.purge_cache {
        let cache = cache_compose_dir()?;
        if cache.exists() {
            fs::remove_dir_all(&cache).map_err(|e| {
                ClientError::Other(format!("Failed to remove {}: {e}", cache.display()))
            })?;
            cache_purged = true;
        }
    }

    let report = UninstallReport {
        compose_dir: compose_dir.display().to_string(),
        compose_dir_source: source.as_str().to_string(),
        volumes_purged: args.purge,
        cache_purged,
    };

    if json {
        output::print_json_typed(&report);
    } else {
        print_human(&report, source);
    }
    Ok(())
}

fn print_human(r: &UninstallReport, source: ComposeDirSource) {
    println!("Docker uninstall");
    println!("================");
    println!("Work dir:      {} ({})", r.compose_dir, source.as_str());
    println!("Volumes purged: {}", r.volumes_purged);
    println!("Cache purged:   {}", r.cache_purged);
}
