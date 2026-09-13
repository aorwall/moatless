//! `moat k8s init` — generate a `values.local.yaml` from prompts and provider
//! presets.

use std::path::Path;

use dialoguer::{Input, Select, theme::ColorfulTheme};
use serde::Deserialize;
use serde_json::{Value, json};

use super::cluster::{DEFAULT_NAMESPACE, DEFAULT_VALUES_FILE, KubeTarget};
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

    let handoff = args
        .infrastructure_handoff
        .as_deref()
        .map(load_handoff)
        .transpose()?;
    if let Some(handoff) = &handoff {
        if args
            .provider
            .is_some_and(|provider| provider != Provider::Eks)
        {
            return Err(ClientError::Other(
                "Infrastructure handoff provider cannot be overridden".to_string(),
            ));
        }
        validate_handoff_override(
            "namespace",
            args.namespace.as_deref(),
            &handoff.app_namespace,
        )?;
        validate_handoff_override(
            "sandbox namespace",
            args.sandbox_namespace.as_deref(),
            &handoff.sandbox_namespace,
        )?;
        validate_handoff_override("domain", args.domain.as_deref(), &handoff.app_domain)?;
        validate_handoff_override(
            "proxy domain",
            args.proxy_domain.as_deref(),
            &handoff.preview_domain,
        )?;
        validate_handoff_override("image tag", args.tag.as_deref(), &handoff.image_tag)?;
        validate_handoff_override("context", args.context.as_deref(), &handoff.context)?;
        let selected_context = match args.context.as_deref() {
            Some(context) => context.to_string(),
            None => super::cluster::current_context().await?.unwrap_or_default(),
        };
        if selected_context != handoff.context {
            return Err(ClientError::Other(format!(
                "Infrastructure handoff expects kubectl context '{}'; pass `--context {}` or select it as the current context",
                handoff.context, handoff.context,
            )));
        }
    }

    let target = KubeTarget::new(args.context.clone());
    let provider = resolve_provider(
        args.provider.or(handoff.as_ref().map(|_| Provider::Eks)),
        &target,
    )
    .await?;
    let namespace = prompt_str(
        args.namespace,
        "Application namespace",
        handoff
            .as_ref()
            .map_or(DEFAULT_NAMESPACE, |value| value.app_namespace.as_str()),
    )?;
    let sandbox_namespace = prompt_str(
        args.sandbox_namespace,
        "Sandbox namespace",
        handoff
            .as_ref()
            .map_or(namespace.as_str(), |value| value.sandbox_namespace.as_str()),
    )?;
    let domain = prompt_str(
        args.domain,
        "Main domain (e.g. moatless.example.com)",
        handoff
            .as_ref()
            .map_or("moatless.example.com", |value| value.app_domain.as_str()),
    )?;
    let proxy_domain = prompt_str(
        args.proxy_domain,
        "Wildcard preview domain (e.g. preview.example.com)",
        handoff
            .as_ref()
            .map_or("preview.example.com", |value| value.preview_domain.as_str()),
    )?;
    let tag = prompt_str(
        args.tag,
        "Image tag",
        handoff
            .as_ref()
            .map_or("latest", |value| value.image_tag.as_str()),
    )?;
    let auth_choice = resolve_auth(args.auth)?;

    let values = build_values(&ValuesInput {
        provider,
        namespace: &namespace,
        sandbox_namespace: &sandbox_namespace,
        domain: &domain,
        proxy_domain: &proxy_domain,
        tag: &tag,
        auth: auth_choice,
        handoff: handoff.as_ref(),
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
            "sandboxNamespace": sandbox_namespace,
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

fn validate_handoff_override(
    name: &str,
    explicit: Option<&str>,
    expected: &str,
) -> Result<(), ClientError> {
    if explicit.is_some_and(|value| value != expected) {
        return Err(ClientError::Other(format!(
            "Infrastructure handoff {name} cannot be overridden; expected '{expected}'"
        )));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InfrastructureHandoff {
    schema_version: u32,
    provider: String,
    compute_mode: String,
    context: String,
    app_namespace: String,
    sandbox_namespace: String,
    region: String,
    app_domain: String,
    preview_domain: String,
    certificate_arn: String,
    workspace_bucket: String,
    backend_role_arn: String,
    database_secret_arn: String,
    database_endpoint: String,
    database_name: String,
    database_port: u16,
    cache_endpoint: String,
    provisioning_method: String,
    image_tag: String,
}

fn load_handoff(path: &str) -> Result<InfrastructureHandoff, ClientError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| ClientError::Other(format!("Failed to read {path}: {error}")))?;
    parse_handoff(&raw)
}

fn parse_handoff(raw: &str) -> Result<InfrastructureHandoff, ClientError> {
    let handoff: InfrastructureHandoff = serde_json::from_str(raw)
        .map_err(|error| ClientError::Other(format!("Invalid infrastructure handoff: {error}")))?;
    if handoff.schema_version != 1
        || handoff.provider != "aws"
        || handoff.compute_mode != "fargate"
        || !matches!(handoff.provisioning_method.as_str(), "terraform" | "cli")
    {
        return Err(ClientError::Other(
            "Unsupported infrastructure handoff; expected schemaVersion=1, provider=aws, computeMode=fargate, and provisioningMethod=terraform|cli"
                .to_string(),
        ));
    }
    for (name, value) in [
        ("context", handoff.context.as_str()),
        ("appNamespace", handoff.app_namespace.as_str()),
        ("sandboxNamespace", handoff.sandbox_namespace.as_str()),
        ("region", handoff.region.as_str()),
        ("appDomain", handoff.app_domain.as_str()),
        ("previewDomain", handoff.preview_domain.as_str()),
        ("certificateArn", handoff.certificate_arn.as_str()),
        ("workspaceBucket", handoff.workspace_bucket.as_str()),
        ("backendRoleArn", handoff.backend_role_arn.as_str()),
        ("databaseSecretArn", handoff.database_secret_arn.as_str()),
        ("databaseEndpoint", handoff.database_endpoint.as_str()),
        ("databaseName", handoff.database_name.as_str()),
        ("cacheEndpoint", handoff.cache_endpoint.as_str()),
        ("imageTag", handoff.image_tag.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(ClientError::Other(format!(
                "Infrastructure handoff field '{name}' must not be empty"
            )));
        }
    }
    if handoff.database_port == 0 {
        return Err(ClientError::Other(
            "Infrastructure handoff field 'databasePort' must be greater than zero".to_string(),
        ));
    }
    Ok(handoff)
}

struct ValuesInput<'a> {
    provider: Provider,
    namespace: &'a str,
    sandbox_namespace: &'a str,
    domain: &'a str,
    proxy_domain: &'a str,
    tag: &'a str,
    auth: AuthChoice,
    handoff: Option<&'a InfrastructureHandoff>,
}

#[derive(Copy, Clone, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum AuthChoice {
    LocalPassword,
    GithubApp,
    Auth0,
}

impl AuthChoice {
    fn from_str(s: &str) -> Result<Self, ClientError> {
        match s.trim().to_ascii_lowercase().as_str() {
            // `none`/`disabled` also select local username/password: there is
            // no separate no-auth mode.
            "" | "local" | "password" | "none" | "off" | "disabled" => {
                Ok(AuthChoice::LocalPassword)
            }
            "github_app" | "github-app" | "gh-app" => Ok(AuthChoice::GithubApp),
            "auth0" => Ok(AuthChoice::Auth0),
            other => Err(ClientError::Other(format!(
                "Unknown auth provider '{other}'. Expected one of: local, github_app, auth0."
            ))),
        }
    }
}

fn resolve_auth(explicit: Option<String>) -> Result<AuthChoice, ClientError> {
    if let Some(s) = explicit {
        return AuthChoice::from_str(&s);
    }
    let choices = ["local", "github_app", "auth0"];
    let idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Auth provider")
        .items(choices)
        .default(0)
        .interact()
        .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))?;
    AuthChoice::from_str(choices[idx])
}

async fn resolve_provider(
    explicit: Option<Provider>,
    target: &KubeTarget,
) -> Result<Provider, ClientError> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    // Try to auto-detect; fall back to prompt.
    let detected = detect_provider_from_cluster(target).await.ok().flatten();
    if let Some(p) = detected {
        println!("Detected provider: {}", p.as_str());
        return Ok(p);
    }
    let choices = ["aks", "eks", "generic"];
    let idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Target provider")
        .items(choices)
        .default(2)
        .interact()
        .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))?;
    Ok(match idx {
        0 => Provider::Aks,
        1 => Provider::Eks,
        _ => Provider::Generic,
    })
}

async fn detect_provider_from_cluster(
    target: &KubeTarget,
) -> Result<Option<Provider>, ClientError> {
    let Some(v) = target.kubectl_json(&["get", "nodes", "-o", "json"]).await? else {
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
        AuthChoice::LocalPassword => {
            auth_block["provider"] = json!("local");
        }
        AuthChoice::GithubApp => {
            auth_block["provider"] = json!("github_app");
            auth_block["githubApp"] = json!({
                "enabled": true,
                "callbackUrl": format!("https://{}/api/v1/auth/github-app/callback", input.domain),
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

    let mut values = json!({
        "image": { "tag": input.tag },
        "sandboxImage": { "tag": input.tag, "pullPolicy": "Always" },
        "backend": { "image": { "tag": input.tag } },
        "t3": {
            "enabled": true,
            "host": "",
            "image": { "tag": input.tag },
        },
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
            // Unconditional: `/verify` authenticates in every mode, so
            // gating this would leave preview routes open under local auth.
            "traefikMiddleware": { "enabled": true },
        },
        "auth": auth_block,
        "database": { "urlSecretKey": "DATABASE_URL" },
        "redis": { "enabled": true },
        "kubernetes": {
            "enabled": true,
            "namespace": input.sandbox_namespace,
            "createNamespace": false,
        },
        "secrets": { "create": false },
        "externalSecrets": {
            "enabled": true,
            "secretName": super::cluster::SECRET_NAME,
        },
    });

    if let Some(handoff) = input.handoff {
        let alb_group = format!("{}-moatless", input.namespace);
        let annotations = json!({
            "alb.ingress.kubernetes.io/scheme": "internet-facing",
            "alb.ingress.kubernetes.io/target-type": "ip",
            "alb.ingress.kubernetes.io/listen-ports": "[{\"HTTP\":80},{\"HTTPS\":443}]",
            "alb.ingress.kubernetes.io/ssl-redirect": "443",
            "alb.ingress.kubernetes.io/certificate-arn": handoff.certificate_arn,
            "alb.ingress.kubernetes.io/group.name": alb_group,
            "alb.ingress.kubernetes.io/healthcheck-path": "/",
        });
        let mut proxy_annotations = annotations.clone();
        proxy_annotations["alb.ingress.kubernetes.io/healthcheck-path"] = json!("/ping");
        proxy_annotations["alb.ingress.kubernetes.io/healthcheck-port"] = json!("8080");
        values["t3"] = json!({
            "enabled": true,
            "host": "",
            "image": { "tag": input.tag },
            "replicaCount": 1,
            "resources": equal_resources("200m", "128Mi"),
        });
        values["backend"]["replicas"] = json!(1);
        values["backend"]["resources"] = equal_resources("2", "1.5Gi");
        values["ingress"]["annotations"] = annotations.clone();
        values["proxyIngress"]["annotations"] = proxy_annotations;
        values["proxyIngress"]["tls"] = Value::Null;
        values["proxy"]["traefikEntryPoint"] = json!("web");
        values["serviceAccount"] = json!({
            "create": true,
            "name": "moatless-backend",
            "annotations": {
                "eks.amazonaws.com/role-arn": handoff.backend_role_arn,
            },
        });
        values["storage"] = json!({
            "backend": "s3",
            "s3": {
                "bucketName": handoff.workspace_bucket,
                "region": handoff.region,
                "keyPrefix": "uploads",
                "createSharedVolume": false,
            },
            "sandboxMountPath": "/opt/moatless/workspace/uploads",
        });
        values["redis"] = json!({
            "enabled": false,
            "urlSecretKey": "REDIS_URL",
        });
        values["kubernetes"] = json!({
            "enabled": true,
            "computeMode": "fargate",
            "namespace": input.sandbox_namespace,
            "createNamespace": false,
            "sandboxRunAsNonRoot": true,
            "sandboxEphemeralStorage": "40Gi",
            "podStartupTimeout": 900,
            "sandboxResources": equal_resources("4", "16Gi"),
            "serverResources": equal_resources("1000m", "1536Mi"),
            "sandboxStorage": {
                "enabled": false,
                "size": "3Gi",
                "storageClass": "",
                "defaultMode": "s3",
            },
            "imagePrepull": { "enabled": false },
            "cacheVolumes": [],
            "nodeArchive": { "enabled": false },
        });
        values["networkPolicy"] = json!({ "sandbox": { "enabled": false } });
    }

    values
}

fn equal_resources(cpu: &str, memory: &str) -> Value {
    json!({
        "limits": { "cpu": cpu, "memory": memory },
        "requests": { "cpu": cpu, "memory": memory },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HANDOFF: &str = r#"{
        "schemaVersion": 1,
        "provider": "aws",
        "computeMode": "fargate",
        "context": "moatless-dev-eu-north-1",
        "appNamespace": "moatless",
        "sandboxNamespace": "moatless-sandboxes",
        "region": "eu-north-1",
        "appDomain": "moatless.example.com",
        "previewDomain": "preview.example.com",
        "certificateArn": "arn:aws:acm:eu-north-1:123456789012:certificate/test",
        "workspaceBucket": "moatless-dev-workspaces-123456789012",
        "backendRoleArn": "arn:aws:iam::123456789012:role/moatless-backend",
        "databaseSecretArn": "arn:aws:secretsmanager:eu-north-1:123456789012:secret:test",
        "databaseEndpoint": "db.example.amazonaws.com",
        "databaseName": "moatless",
        "databasePort": 5432,
        "cacheEndpoint": "cache.example.amazonaws.com",
        "provisioningMethod": "terraform",
        "imageTag": "latest"
    }"#;

    #[test]
    fn parses_versioned_aws_fargate_handoff() {
        let handoff = parse_handoff(HANDOFF).unwrap();
        assert_eq!(handoff.schema_version, 1);
        assert_eq!(handoff.image_tag, "latest");
    }

    #[test]
    fn rejects_handoff_owned_field_override() {
        let error = validate_handoff_override("namespace", Some("another-namespace"), "moatless")
            .unwrap_err();
        assert!(error.to_string().contains("cannot be overridden"));
        assert!(validate_handoff_override("namespace", Some("moatless"), "moatless").is_ok());
    }

    #[test]
    fn fargate_handoff_generates_s3_only_values() {
        let handoff = parse_handoff(HANDOFF).unwrap();
        let values = build_values(&ValuesInput {
            provider: Provider::Eks,
            namespace: "moatless",
            sandbox_namespace: "moatless-sandboxes",
            domain: "moatless.example.com",
            proxy_domain: "preview.example.com",
            tag: &handoff.image_tag,
            auth: AuthChoice::LocalPassword,
            handoff: Some(&handoff),
        });

        assert_eq!(values.pointer("/storage/backend"), Some(&json!("s3")));
        assert_eq!(
            values.pointer("/storage/s3/createSharedVolume"),
            Some(&json!(false))
        );
        assert_eq!(
            values.pointer("/kubernetes/computeMode"),
            Some(&json!("fargate"))
        );
        assert_eq!(
            values.pointer("/kubernetes/imagePrepull/enabled"),
            Some(&json!(false))
        );
        assert_eq!(
            values.pointer("/kubernetes/sandboxStorage/defaultMode"),
            Some(&json!("s3"))
        );
        assert_eq!(values.pointer("/redis/enabled"), Some(&json!(false)));
        assert_eq!(
            values.pointer("/redis/urlSecretKey"),
            Some(&json!("REDIS_URL"))
        );
        assert_eq!(values.pointer("/image/tag"), Some(&json!("latest")));
        assert_eq!(values.pointer("/sandboxImage/tag"), Some(&json!("latest")));
        assert_eq!(
            values.pointer("/sandboxImage/pullPolicy"),
            Some(&json!("Always"))
        );
        assert_eq!(values.pointer("/backend/image/tag"), Some(&json!("latest")));
        assert_eq!(values.pointer("/t3/enabled"), Some(&json!(true)));
        assert_eq!(values.pointer("/t3/image/tag"), Some(&json!("latest")));
        assert_eq!(
            values.pointer("/serviceAccount/name"),
            Some(&json!("moatless-backend"))
        );
        assert_eq!(
            values.pointer("/networkPolicy/sandbox/enabled"),
            Some(&json!(false))
        );
        assert_eq!(
            values.pointer("/ingress/annotations/alb.ingress.kubernetes.io~1healthcheck-path"),
            Some(&json!("/"))
        );
        assert_eq!(
            values.pointer("/proxyIngress/annotations/alb.ingress.kubernetes.io~1healthcheck-path"),
            Some(&json!("/ping"))
        );
    }
}
