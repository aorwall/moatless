//! `moat k8s uninstall` — helm uninstall + optional cleanup.

use dialoguer::{Confirm, theme::ColorfulTheme};
use serde_json::json;
use std::io::IsTerminal;

use super::UninstallArgs;
use super::cluster::{KubeTarget, SECRET_NAME, require_binary};
use crate::client::ClientError;
use crate::output;

pub(super) async fn run(args: UninstallArgs, json_out: bool) -> Result<(), ClientError> {
    require_binary("helm")?;
    require_binary("kubectl")?;
    let target = KubeTarget::new(args.context.clone());

    if std::io::stdin().is_terminal() && !json_out {
        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(format!(
                "Uninstall release '{}' from namespace '{}' in context '{}'{}?",
                args.release,
                args.namespace,
                target.context().unwrap_or("<current>"),
                if args.purge {
                    " (and purge Secret + PVCs)"
                } else {
                    ""
                }
            ))
            .default(false)
            .interact()
            .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))?;
        if !confirmed {
            return Err(ClientError::Other("Aborted.".into()));
        }
    }

    let out = target
        .helm(&["uninstall", &args.release, "--namespace", &args.namespace])
        .await?;
    if !out.success() {
        return Err(ClientError::Other(format!(
            "helm uninstall failed: {}",
            out.stderr.trim()
        )));
    }

    let mut purged = Vec::new();
    let orphaned_pvcs = list_pvcs(&target, &args.namespace).await?;

    if args.purge {
        // Delete the app secret (best-effort).
        let _ = target
            .kubectl(&[
                "-n",
                &args.namespace,
                "delete",
                "secret",
                SECRET_NAME,
                "--ignore-not-found",
            ])
            .await;
        purged.push(format!("secret/{SECRET_NAME}"));

        for pvc in &orphaned_pvcs {
            let _ = target
                .kubectl(&[
                    "-n",
                    &args.namespace,
                    "delete",
                    "pvc",
                    pvc,
                    "--ignore-not-found",
                ])
                .await;
            purged.push(format!("pvc/{pvc}"));
        }
    }

    if json_out {
        output::print_json_typed(&json!({
            "uninstalled": args.release,
            "context": target.context(),
            "namespace": args.namespace,
            "purged": purged,
            "orphanedPvcs": if args.purge { Vec::<String>::new() } else { orphaned_pvcs },
        }));
    } else {
        println!(
            "Uninstalled release {} from namespace {}.",
            args.release, args.namespace
        );
        if args.purge {
            if purged.is_empty() {
                println!("No additional resources to purge.");
            } else {
                println!("Purged: {}", purged.join(", "));
            }
        } else if !orphaned_pvcs.is_empty() {
            println!();
            println!("Orphaned PVCs remain (pass --purge next time to delete):");
            for p in &orphaned_pvcs {
                println!("  - {p}");
            }
        }
    }

    Ok(())
}

async fn list_pvcs(target: &KubeTarget, namespace: &str) -> Result<Vec<String>, ClientError> {
    let Some(v) = target
        .kubectl_json(&["-n", namespace, "get", "pvc", "-o", "json"])
        .await?
    else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
        for item in items {
            if let Some(name) = item.pointer("/metadata/name").and_then(|n| n.as_str()) {
                out.push(name.to_string());
            }
        }
    }
    Ok(out)
}
