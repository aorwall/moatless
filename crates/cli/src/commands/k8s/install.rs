//! `moat k8s install` / `upgrade` — helm wrapper with sensible defaults.

use std::path::Path;

use serde_json::json;

use super::helpers::{helm, require_binary, run_streaming};
use super::{InstallArgs, UpgradeArgs};
use crate::client::ClientError;
use crate::output;

pub(super) async fn run(args: InstallArgs, json_out: bool) -> Result<(), ClientError> {
    require_binary("helm")?;
    require_binary("kubectl")?;

    if !Path::new(&args.chart).exists() {
        return Err(ClientError::Other(format!(
            "Chart path '{}' does not exist. Run from the repo root or pass --chart.",
            args.chart
        )));
    }
    if !Path::new(&args.values).exists() {
        return Err(ClientError::Other(format!(
            "Values file '{}' does not exist. Run `moat k8s init` first.",
            args.values
        )));
    }

    if !args.skip_deps {
        println!("→ helm dependency update {}", args.chart);
        let out = helm(&["dependency", "update", &args.chart]).await?;
        if !out.success() {
            return Err(ClientError::Other(format!(
                "helm dependency update failed: {}",
                out.stderr.trim()
            )));
        }
    }

    let mut cmd: Vec<String> = vec![
        "upgrade".into(),
        "--install".into(),
        args.release.clone(),
        args.chart.clone(),
        "--namespace".into(),
        args.namespace.clone(),
        "--create-namespace".into(),
        "--values".into(),
        args.values.clone(),
    ];
    if args.dry_run {
        cmd.push("--dry-run".into());
    }
    if args.wait {
        cmd.push("--wait".into());
        cmd.push("--timeout".into());
        cmd.push("10m".into());
    }

    if json_out {
        // Machine-readable mode — capture output.
        let refs: Vec<&str> = cmd.iter().map(String::as_str).collect();
        let out = helm(&refs).await?;
        let report = json!({
            "success": out.success(),
            "release": args.release,
            "namespace": args.namespace,
            "dryRun": args.dry_run,
            "stdout": out.stdout,
            "stderr": out.stderr,
        });
        output::print_json_typed(&report);
        if !out.success() {
            return Err(ClientError::Other("helm upgrade --install failed".into()));
        }
    } else {
        let refs: Vec<&str> = cmd.iter().map(String::as_str).collect();
        println!("→ helm {}", refs.join(" "));
        run_streaming("helm", &refs).await?;
        println!();
        println!("Done. Run `moat k8s status` to verify the rollout.");
    }

    Ok(())
}

pub(super) async fn upgrade(args: UpgradeArgs, json_out: bool) -> Result<(), ClientError> {
    require_binary("helm")?;

    let set_tag = format!("image.tag={}", args.tag);
    let set_sandbox = format!("sandboxImage.tag={}", args.tag);
    let cmd = vec![
        "upgrade".to_string(),
        args.release.clone(),
        "--namespace".to_string(),
        args.namespace.clone(),
        "--reuse-values".to_string(),
        "--set".to_string(),
        set_tag,
        "--set".to_string(),
        set_sandbox,
    ];
    let refs: Vec<&str> = cmd.iter().map(String::as_str).collect();

    if json_out {
        let out = helm(&refs).await?;
        let report = json!({
            "success": out.success(),
            "release": args.release,
            "namespace": args.namespace,
            "tag": args.tag,
            "stdout": out.stdout,
            "stderr": out.stderr,
        });
        output::print_json_typed(&report);
        if !out.success() {
            return Err(ClientError::Other(out.stderr));
        }
    } else {
        println!("→ helm {}", refs.join(" "));
        run_streaming("helm", &refs).await?;
        println!();
        println!("Rolled out {}. Run `moat k8s status` to verify.", args.tag);
    }
    Ok(())
}
