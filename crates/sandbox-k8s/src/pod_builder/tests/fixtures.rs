//! The config, plan and pod builders every case in this subtree starts from.

use super::*;

/// Build a `ResourceQuantities` from optional quantity spellings.
pub(super) fn quantities(cpu: Option<&str>, memory: Option<&str>) -> ResourceQuantities {
    ResourceQuantities {
        cpu: cpu.map(|q| q.parse().expect("cpu quantity")),
        memory: memory.map(|q| q.parse().expect("memory quantity")),
    }
}

pub(super) fn memory(quantity: &str) -> Memory {
    quantity.parse().expect("memory quantity")
}

pub(super) fn pod_with_env(env: Vec<EnvVar>) -> Pod {
    Pod {
        spec: Some(PodSpec {
            containers: vec![Container {
                name: "sandbox".to_string(),
                env: Some(env),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Literal value of a container env var, or `None` when it is unset.
pub(super) fn env_value<'a>(container: &'a Container, name: &str) -> Option<&'a str> {
    container
        .env
        .as_ref()?
        .iter()
        .find(|e| e.name == name)
        .and_then(|e| e.value.as_deref())
}

pub(super) fn test_config() -> Config {
    let mut config = Config::test_minimal();
    config.sandbox.image = "sandbox:latest".into();
    config.sandbox.task_token_secret = Some("secret".into());
    config
}

pub(super) fn test_plan() -> ProvisionPlan {
    ProvisionPlan {
        repo_config: RepoConfig {
            name: "repo".into(),
            docker_image: None,
            system_prompt: None,
            code_execution: None,
            resources: None,
            plugins: None,
            docker_access: None,
            docker_networks: None,
            sandbox_service_account: None,
            network_profile: None,
            node_selector: None,
        },
        git_clone_url: "https://github.com/test/repo.git".into(),
        git_branch: None,
        git_provider: "github".into(),
        env_vars: None,
        repos: vec![RepoCheckout {
            uses_ssh: false,
            repository_id: "repo-1".into(),
            clone_url: "https://github.com/test/repo.git".into(),
            branch: None,
            provider: "github".into(),
            mount_name: "repo".into(),
            is_primary: true,
        }],
    }
}

/// `(cpu_request, memory_request, cpu_limit, memory_limit)` off a container.
pub(super) fn resource_quantities(container: &Container) -> (&str, &str, &str, &str) {
    let resources = container.resources.as_ref().expect("resources are set");
    let requests = resources.requests.as_ref().expect("requests are set");
    let limits = resources.limits.as_ref().expect("limits are set");
    (
        requests["cpu"].0.as_str(),
        requests["memory"].0.as_str(),
        limits["cpu"].0.as_str(),
        limits["memory"].0.as_str(),
    )
}

pub(super) fn sandbox_env(pod: &Pod) -> Vec<k8s_openapi::api::core::v1::EnvVar> {
    pod.spec
        .as_ref()
        .unwrap()
        .containers
        .iter()
        .find(|c| c.name == "sandbox")
        .unwrap()
        .env
        .clone()
        .unwrap()
}

pub(super) fn fargate_config() -> Config {
    let mut config = test_config();
    config.sandbox.compute_mode = moatless_core::config::KubernetesComputeMode::Fargate;
    config.sandbox.default_pvc_enabled = false;
    config.sandbox.default_s3_enabled = true;
    config.sandbox.run_as_non_root = true;
    config.sandbox.ephemeral_storage = Some("40Gi".into());
    config.sandbox.sandbox_resources_cpu_request = "4".into();
    config.sandbox.sandbox_resources_cpu_limit = "4".into();
    config.sandbox.sandbox_resources_memory_request = "16Gi".into();
    config.sandbox.sandbox_resources_memory_limit = "16Gi".into();
    config.sandbox.resources_cpu_request = "1000m".into();
    config.sandbox.resources_cpu_limit = "1000m".into();
    config.sandbox.resources_memory_request = "1536Mi".into();
    config.sandbox.resources_memory_limit = "1536Mi".into();
    config
}

pub(super) fn only_match_field(term: &NodeSelectorTerm) -> (&str, &str) {
    let requirement = &term.match_fields.as_ref().expect("match fields")[0];
    (
        requirement.key.as_str(),
        requirement.values.as_ref().expect("values")[0].as_str(),
    )
}
