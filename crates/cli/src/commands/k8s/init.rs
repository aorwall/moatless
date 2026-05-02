//! `moat k8s init` — generate a `values.local.yaml` from prompts and provider presets.

use std::path::Path;

use dialoguer::{theme::ColorfulTheme, Input, Select};
use serde_json::json;

use super::helpers::{DEFAULT_NAMESPACE, DEFAULT_VALUES_FILE};
use super::{InitArgs, Provider};
use crate::client::ClientError;

pub(super) async fn run(args: InitArgs, json_out: bool) -> Result<(), ClientError> {
    let output_path = args
        .output
        .clone()
        .unwrap_or_else(|| DEFAULT_VALUES_FILE.to_string());

    if Path::new(&output_path).exists() && !args.force {
        return Err(ClientError::Other(format!(
            "{output_path} already exists. Pass --force to overwrite."
        )));
    }

    let provider = resolve_provider(args.provider).await?;
    let namespace = prompt_str(args.namespace, "Namespace", DEFAULT_NAMESPACE)?;
    let domain = prompt_str(
        args.domain,
        "Main domain (e.g. vibe.example.com)",
        "vibe.example.com",
    )?;
    let proxy_domain = prompt_str(
        args.proxy_domain,
        "Wildcard preview domain (e.g. preview.example.com)",
        "preview.example.com",
    )?;
    let tag = prompt_str(args.tag, "Image tag", "latest")?;
    let auth_choice = resolve_auth(args.auth)?;

    let values = build_values(&ValuesInput {
        provider,
        namespace: &namespace,
        domain: &domain,
        proxy_domain: &proxy_domain,
        tag: &tag,
        auth: auth_choice,
    });

    let yaml = serde_yaml::to_string(&values)
        .map_err(|e| ClientError::Other(format!("Failed to serialize YAML: {e}")))?;

    std::fs::write(&output_path, &yaml)
        .map_err(|e| ClientError::Other(format!("Failed to write {output_path}: {e}")))?;

    if json_out {
        crate::output::print_json_typed(&json!({
            "output": output_path,
            "provider": provider.as_str(),
            "namespace": namespace,
            "domain": domain,
            "proxyDomain": proxy_domain,
            "tag": tag,
            "auth": auth_choice,
        }));
    } else {
        println!("Wrote {output_path} ({} bytes).", yaml.len());
        println!();
        println!("Next steps:");
        println!("  1. `moat k8s secrets bootstrap`   — collect API keys and DB URL");
        println!("  2. `moat k8s install`             — helm upgrade --install");
        println!("  3. `moat k8s status`              — verify the rollout");
    }

    Ok(())
}

struct ValuesInput<'a> {
    provider: Provider,
    namespace: &'a str,
    domain: &'a str,
    proxy_domain: &'a str,
    tag: &'a str,
    auth: AuthChoice,
}

#[derive(Copy, Clone, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum AuthChoice {
    None,
    Github,
    Auth0,
}

impl AuthChoice {
    fn from_str(s: &str) -> Result<Self, ClientError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "none" | "off" | "disabled" => Ok(AuthChoice::None),
            "github" | "gh" => Ok(AuthChoice::Github),
            "auth0" => Ok(AuthChoice::Auth0),
            other => Err(ClientError::Other(format!(
                "Unknown auth provider '{other}'. Expected one of: none, github, auth0."
            ))),
        }
    }
}

fn resolve_auth(explicit: Option<String>) -> Result<AuthChoice, ClientError> {
    if let Some(s) = explicit {
        return AuthChoice::from_str(&s);
    }
    let choices = ["none", "github", "auth0"];
    let idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Auth provider")
        .items(&choices)
        .default(0)
        .interact()
        .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))?;
    AuthChoice::from_str(choices[idx])
}

async fn resolve_provider(explicit: Option<Provider>) -> Result<Provider, ClientError> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    // Try to auto-detect; fall back to prompt.
    let detected = detect_provider_from_cluster().await.ok().flatten();
    if let Some(p) = detected {
        println!("Detected provider: {}", p.as_str());
        return Ok(p);
    }
    let choices = ["aks", "eks", "generic"];
    let idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Target provider")
        .items(&choices)
        .default(2)
        .interact()
        .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))?;
    Ok(match idx {
        0 => Provider::Aks,
        1 => Provider::Eks,
        _ => Provider::Generic,
    })
}

async fn detect_provider_from_cluster() -> Result<Option<Provider>, ClientError> {
    use super::helpers::kubectl_json;
    let Some(v) = kubectl_json(&["get", "nodes", "-o", "json"]).await? else {
        return Ok(None);
    };
    let Some(items) = v.get("items").and_then(|i| i.as_array()) else {
        return Ok(None);
    };
    for node in items {
        let provider_id = node
            .pointer("/spec/providerID")
            .and_then(|p| p.as_str())
            .unwrap_or("");
        if provider_id.starts_with("azure://") {
            return Ok(Some(Provider::Aks));
        }
        if provider_id.starts_with("aws://") {
            return Ok(Some(Provider::Eks));
        }
    }
    Ok(None)
}

fn prompt_str(
    explicit: Option<String>,
    prompt: &str,
    default: &str,
) -> Result<String, ClientError> {
    if let Some(s) = explicit {
        return Ok(s);
    }
    Input::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .default(default.to_string())
        .interact_text()
        .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))
}

fn build_values(input: &ValuesInput<'_>) -> serde_json::Value {
    let (ingress_class, ingress_annotations) = match input.provider {
        Provider::Aks => ("webapprouting.kubernetes.azure.com", json!({})),
        Provider::Eks => (
            "alb",
            json!({
                "alb.ingress.kubernetes.io/scheme": "internet-facing",
                "alb.ingress.kubernetes.io/target-type": "ip",
                "alb.ingress.kubernetes.io/listen-ports": "[{\"HTTP\":80},{\"HTTPS\":443}]",
                "alb.ingress.kubernetes.io/ssl-redirect": "443",
            }),
        ),
        Provider::Generic => ("nginx", json!({})),
    };

    let mut auth_block = json!({
        "provider": "",
        "frontendUrl": format!("https://{}", input.domain),
    });
    match input.auth {
        AuthChoice::None => {}
        AuthChoice::Github => {
            auth_block["provider"] = json!("github");
            auth_block["github"] = json!({
                "enabled": true,
                "callbackUrl": format!("https://{}/api/auth/github/callback", input.domain),
            });
        }
        AuthChoice::Auth0 => {
            auth_block["provider"] = json!("auth0");
            auth_block["auth0"] = json!({
                "enabled": true,
                "callbackUrl": format!("https://{}/api/auth/auth0/callback", input.domain),
            });
        }
    }

    json!({
        "image": { "tag": input.tag },
        "sandboxImage": { "tag": input.tag },
        "ingress": {
            "enabled": true,
            "className": ingress_class,
            "annotations": ingress_annotations,
            "hosts": [
                { "host": input.domain, "paths": [{ "path": "/", "pathType": "Prefix" }] }
            ],
            "tls": json!([]),
        },
        "proxyIngress": {
            "enabled": true,
            "className": ingress_class,
            "host": format!("*.{}", input.proxy_domain),
            "tls": {
                "secretName": "proxy-wildcard-tls",
                "hosts": [format!("*.{}", input.proxy_domain)],
            },
        },
        "proxy": {
            "domain": input.proxy_domain,
            "protocol": "https",
            "traefikIngressRoute": { "enabled": true },
            "traefikMiddleware": { "enabled": matches!(input.auth, AuthChoice::Github | AuthChoice::Auth0) },
        },
        "auth": auth_block,
        "database": { "urlSecretKey": "DATABASE_URL" },
        "redis": { "enabled": true },
        "kubernetes": {
            "enabled": true,
            "namespace": input.namespace,
            "createNamespace": false,
        },
        "secrets": { "create": false },
        "externalSecrets": {
            "enabled": true,
            "secretName": super::helpers::SECRET_NAME,
        },
    })
}
