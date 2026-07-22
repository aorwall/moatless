//! `moat docker install` — `docker compose up -d`, with optional pull + wait.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::helpers::{
    detect_compose_cmd, resolve_compose_dir, run_compose_streaming, ComposeDirSource, ENV_FILE,
};
use super::InstallArgs;
use crate::client::ClientError;
use crate::output;

#[derive(Serialize, Deserialize)]
pub(super) struct InstallReport {
    pub(super) compose_dir: String,
    pub(super) compose_dir_source: String,
    pub(super) compose_command: String,
    pub(super) pulled: bool,
    pub(super) up_exit_code: i32,
    pub(super) waited: bool,
    pub(super) all_healthy: Option<bool>,
    pub(super) stderr: Option<String>,
}

pub(super) async fn run(args: InstallArgs, json: bool) -> Result<(), ClientError> {
    let (compose_dir, source) = resolve_compose_dir(args.compose_dir.as_deref())?;
    let compose = detect_compose_cmd().await?;

    // Sanity: `.env` must exist. Fail loud rather than let compose balk with
    // `variable not set` warnings.
    if !compose_dir.join(ENV_FILE).is_file() {
        return Err(ClientError::Other(format!(
            "{} missing — run `moat docker init` first",
            compose_dir.join(ENV_FILE).display()
        )));
    }

    if args.pull {
        // Streaming so the user sees download progress.
        run_compose_streaming(&compose, &compose_dir, &["pull"]).await?;
    }

    // `up -d` streaming — users expect to see pulled images and startup lines.
    let up_result = run_compose_streaming(&compose, &compose_dir, &["up", "-d"]).await;
    let up_exit_code = i32::from(up_result.is_err());
    if let Err(e) = up_result {
        // Capture the error but try one follow-up `ps` so the report is useful.
        let report = InstallReport {
            compose_dir: compose_dir.display().to_string(),
            compose_dir_source: source.as_str().to_string(),
            compose_command: compose.label().to_string(),
            pulled: args.pull,
            up_exit_code,
            waited: false,
            all_healthy: None,
            stderr: Some(e.to_string()),
        };
        if json {
            output::print_json_typed(&report);
        } else {
            print_human(&report, source);
        }
        return Err(e);
    }

    let all_healthy = if args.wait {
        Some(wait_for_healthy(&compose_dir, Duration::from_mins(2)).await?)
    } else {
        // Fast `ps` probe to populate the report without blocking.
        super::status::quick_all_healthy(&compose_dir).await.ok()
    };

    let report = InstallReport {
        compose_dir: compose_dir.display().to_string(),
        compose_dir_source: source.as_str().to_string(),
        compose_command: compose.label().to_string(),
        pulled: args.pull,
        up_exit_code,
        waited: args.wait,
        all_healthy,
        stderr: None,
    };

    if json {
        output::print_json_typed(&report);
    } else {
        print_human(&report, source);
    }
    Ok(())
}

async fn wait_for_healthy(
    compose_dir: &std::path::Path,
    timeout: Duration,
) -> Result<bool, ClientError> {
    let deadline = Instant::now() + timeout;
    loop {
        if super::status::quick_all_healthy(compose_dir)
            .await
            .unwrap_or(false)
        {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

fn print_human(r: &InstallReport, source: ComposeDirSource) {
    println!("Docker install");
    println!("==============");
    println!("Work dir:        {} ({})", r.compose_dir, source.as_str());
    println!("Compose command: {}", r.compose_command);
    println!("Pulled:          {}", r.pulled);
    println!("up exit:         {}", r.up_exit_code);
    if r.waited {
        println!(
            "Healthy wait:    {}",
            r.all_healthy
                .map_or("unknown", |h| if h { "all healthy" } else { "timed out" })
        );
    } else if let Some(h) = r.all_healthy {
        println!(
            "Quick health:    {}",
            if h {
                "all healthy"
            } else {
                "some still starting"
            }
        );
    }
    if let Some(err) = &r.stderr {
        println!("Error:           {err}");
    }
    println!();
    println!("Next: `moat docker status` to verify, or `moat docker logs -f --service backend`.");
}
