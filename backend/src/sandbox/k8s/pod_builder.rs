//! Pod spec builder from YAML template with environment injection.

use k8s_openapi::api::core::v1::Pod;
use std::collections::{BTreeMap, HashMap};
use tracing::{debug, info};

use super::naming::generate_pod_name;
use crate::config::Config;
use crate::sandbox::types::ProvisionPlan;

/// Build a Pod spec from a YAML template with task-specific environment injection.
pub fn build_pod_from_template(
    template_content: &str,
    config: &Config,
    task_id: &str,
    plan: &ProvisionPlan,
    pvc_name: Option<&str>,
    secret_name: Option<&str>,
    task_auth_token: &str,
) -> Result<Pod, String> {
    let mut pod: Pod =
        serde_yaml::from_str(template_content).map_err(|e| format!("Failed to parse pod template: {e}"))?;

    let pod_name = generate_pod_name(&plan.repo_config.name, task_id);
    let namespace = config.sandbox_namespace.clone();

    // Set metadata
    pod.metadata.name = Some(pod_name.clone());
    pod.metadata.namespace = Some(namespace.clone());

    // Set labels
    let labels = pod.metadata.labels.get_or_insert_with(BTreeMap::new);
    labels.insert("moatless.io/task-id".to_string(), task_id.to_string());

    // Set annotations
    let annotations = pod.metadata.annotations.get_or_insert_with(BTreeMap::new);
    annotations.insert(
        "moatless.io/repository-url".to_string(),
        plan.git_clone_url.clone(),
    );
    annotations.insert(
        "moatless.io/created-at".to_string(),
        chrono_now_iso(),
    );

    // Build task-specific env vars
    let mut task_env: Vec<serde_json::Value> = vec![
        serde_json::json!({"name": "TASK_ID", "value": task_id}),
        serde_json::json!({"name": "GIT_CLONE_URL", "value": plan.git_clone_url}),
    ];

    if let Some(ref branch) = plan.git_branch {
        task_env.push(serde_json::json!({"name": "GIT_BRANCH", "value": branch}));
    }
    // Build REPO_CONFIG env var from RepoConfig fields
    {
        let rc = &plan.repo_config;
        if rc.system_prompt.is_some()
            || rc.code_execution.is_some()
            || rc.preview_config.is_some()
            || rc.servers.is_some()
            || rc.plugin_ids.is_some()
        {
            // Enrich servers with startCommands from previewConfig
            let enriched_servers: Vec<serde_json::Value> = match &rc.servers {
                Some(servers) => servers
                    .iter()
                    .map(|s| {
                        let mut entry = serde_json::json!({
                            "name": s.name,
                            "port": s.port,
                        });
                        match s.name.as_str() {
                            "files" => {
                                entry["startCommands"] = serde_json::json!(["file-explorer"]);
                            }
                            "preview" => {
                                if let Some(ref pc) = rc.preview_config {
                                    if let Some(cmds) = pc.get("serverStartCommands") {
                                        entry["startCommands"] = cmds.clone();
                                    }
                                    if let Some(timeout) = pc.get("startupTimeoutSeconds") {
                                        entry["startupTimeoutSeconds"] = timeout.clone();
                                    }
                                }
                            }
                            _ => {}
                        }
                        entry
                    })
                    .collect(),
                None => vec![],
            };

            let mut repo_config_map = serde_json::Map::new();
            if let Some(ref sp) = rc.system_prompt {
                repo_config_map.insert("systemPrompt".into(), serde_json::json!(sp));
            }
            if let Some(ref ce) = rc.code_execution {
                repo_config_map.insert("codeExecution".into(), ce.clone());
            }
            if let Some(ref pc) = rc.preview_config {
                repo_config_map.insert("previewConfig".into(), pc.clone());
            }
            if !enriched_servers.is_empty() {
                repo_config_map.insert("servers".into(), serde_json::json!(enriched_servers));
            }
            if let Some(ref pi) = rc.plugin_ids {
                repo_config_map.insert("pluginIds".into(), serde_json::json!(pi));
            }
            let repo_config_value = serde_json::Value::Object(repo_config_map);
            task_env.push(serde_json::json!({
                "name": "REPO_CONFIG",
                "value": repo_config_value.to_string()
            }));
        }
    }

    // Only include credentials inline if no secret is provided
    if secret_name.is_none() {
        if let Some(ref token) = plan.git_provider_token {
            task_env.push(serde_json::json!({"name": "GITHUB_TOKEN", "value": token}));
        }
        if let Some(ref key) = plan.anthropic_api_key {
            task_env.push(serde_json::json!({"name": "ANTHROPIC_API_KEY", "value": key}));
        }
        if let Some(ref token) = plan.claude_oauth_token {
            task_env.push(serde_json::json!({"name": "CLAUDE_CODE_OAUTH_TOKEN", "value": token}));
        }
        task_env.push(serde_json::json!({"name": "TASK_AUTH_TOKEN", "value": task_auth_token}));
    }

    // Custom env vars from repository config
    if let Some(ref env_vars) = plan.env_vars {
        for (key, value) in env_vars {
            task_env.push(serde_json::json!({"name": key, "value": value}));
        }
    }

    // Modify the pod spec: inject env vars into sandbox container
    if let Some(ref mut spec) = pod.spec {
        // Compute startup probe failure threshold
        let startup_timeout = config.k8s_pod_startup_timeout_secs;
        let startup_probe_period: u64 = 5;
        let failure_threshold = (startup_timeout / startup_probe_period) + 6; // +30s buffer

        for container in spec.containers.iter_mut() {
            if container.name == "sandbox" {
                // Override image if specified in repo config (skip empty strings)
                if let Some(ref image) = plan.repo_config.docker_image {
                    if !image.is_empty() {
                        container.image = Some(image.clone());
                    }
                }

                // Append env vars
                let env = container.env.get_or_insert_with(Vec::new);
                for env_var in &task_env {
                    let var: k8s_openapi::api::core::v1::EnvVar =
                        serde_json::from_value(env_var.clone())
                            .map_err(|e| format!("Failed to parse env var: {e}"))?;
                    env.push(var);
                }

                // Add envFrom for secret reference
                if let Some(secret) = secret_name {
                    let env_from = container.env_from.get_or_insert_with(Vec::new);
                    let source: k8s_openapi::api::core::v1::EnvFromSource =
                        serde_json::from_value(serde_json::json!({
                            "secretRef": {"name": secret, "optional": false}
                        }))
                        .map_err(|e| format!("Failed to parse envFrom: {e}"))?;
                    env_from.push(source);
                }

                // Override startup probe
                let probe: k8s_openapi::api::core::v1::Probe =
                    serde_json::from_value(serde_json::json!({
                        "httpGet": {"path": "/health", "port": 8001},
                        "periodSeconds": startup_probe_period,
                        "timeoutSeconds": 5,
                        "failureThreshold": failure_threshold,
                    }))
                    .map_err(|e| format!("Failed to parse startup probe: {e}"))?;
                container.startup_probe = Some(probe);

                // Add PVC volume mount
                if pvc_name.is_some() {
                    let mounts = container.volume_mounts.get_or_insert_with(Vec::new);
                    let mount: k8s_openapi::api::core::v1::VolumeMount =
                        serde_json::from_value(serde_json::json!({
                            "name": "workspace-data",
                            "mountPath": "/opt/moatless",
                        }))
                        .map_err(|e| format!("Failed to parse volume mount: {e}"))?;
                    mounts.push(mount);
                }
            }
        }

        // Add PVC volume
        if let Some(pvc) = pvc_name {
            let volumes = spec.volumes.get_or_insert_with(Vec::new);
            let volume: k8s_openapi::api::core::v1::Volume =
                serde_json::from_value(serde_json::json!({
                    "name": "workspace-data",
                    "persistentVolumeClaim": {"claimName": pvc},
                }))
                .map_err(|e| format!("Failed to parse volume: {e}"))?;
            volumes.push(volume);
        }
    }

    info!(task_id, pod_name = %pod_name, namespace = %namespace, "Pod spec built from template");
    debug!(task_id, env_count = task_env.len(), "Injected environment variables");

    Ok(pod)
}

/// Get current time as ISO 8601 string (no chrono dependency — use simple format)
fn chrono_now_iso() -> String {
    // Use system time to produce a rough ISO timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple epoch seconds — callers only use this for ordering
    format!("{now}Z")
}

/// Build servers name→port map from plan
pub fn build_servers_map(plan: &ProvisionPlan) -> HashMap<String, u16> {
    let mut servers = HashMap::new();
    if let Some(ref server_configs) = plan.repo_config.servers {
        for s in server_configs {
            servers.insert(s.name.clone(), s.port);
        }
    }
    servers
}
