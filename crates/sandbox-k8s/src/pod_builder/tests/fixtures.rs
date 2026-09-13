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
    use moatless_core::config::*;
    use moatless_types::auth::*;
    use moatless_types::sandbox::*;
    Config {
        server: moatless_core::config::ServerConfig {
            port: 8080,
            metrics_port: 9090,
            grpc_port: 50051,
        },
        sandbox_provider: SandboxProviderType::Kubernetes,
        sandbox: SandboxConfig {
            workspace_repository_secret: None,
            workspace_repository_role_arn: None,
            compute_mode: moatless_core::config::KubernetesComputeMode::Standard,
            namespace: "test".into(),
            image: "sandbox:latest".into(),
            image_pull_policy: "Never".into(),
            grpc_port: 50052,
            resources_cpu_limit: "500m".into(),
            resources_memory_limit: "512Mi".into(),
            resources_cpu_request: "100m".into(),
            resources_memory_request: "128Mi".into(),
            sandbox_resources_cpu_request: "1".into(),
            sandbox_resources_cpu_limit: "4".into(),
            sandbox_resources_memory_request: "2Gi".into(),
            sandbox_resources_memory_limit: "8Gi".into(),
            ephemeral_storage: None,
            run_as_non_root: false,
            shared_data_size: "2Gi".into(),
            persist_toolchain: false,
            storage_enabled: false,
            storage_size: "1Gi".into(),
            storage_class: None,
            default_pvc_enabled: true,
            default_s3_enabled: true,
            node_archive_enabled: false,
            retain_storage_on_close: false,
            pod_startup_timeout_secs: 10,
            task_token_secret: Some("secret".into()),
            user_token_ttl_secs: 7 * 24 * 3600,
            bootstrap_token_ttl_secs: 1800,
            emit_legacy_user_token_env: true,
            accept_claimless_tokens: true,
            file_mount_path: "/tmp".into(),
            idle_reaper_enabled: false,
            cron_scheduler_enabled: false,
            liveness_enabled: false,
            idle_timeout_minutes: 15,
            idle_check_interval_secs: 300,
            inactive_task_timeout_days: 0,
            cold_after_hours: 0,
            orphan_gc_enabled: false,
            codex_refresher_interval_secs: 0,
            max_crash_restart_count: 5,
            node_selector_json: None,
            tolerations_json: None,
            service_account_name: None,
            default_model: None,
            default_codex_model: None,
            extra_env: vec![],
            cache_volumes: vec![],
            cache_host_root: "/mnt/cache".into(),
            cache_init_image: "docker:dind".into(),
            docker_dind_image: "docker:dind".into(),
            docker_dind_storage_size: "10Gi".into(),
            docker_dind_cpu_request: "500m".into(),
            docker_dind_cpu_limit: "2".into(),
            docker_dind_memory_request: "1Gi".into(),
            docker_dind_memory_limit: "4Gi".into(),
            egress_proxy_url: None,
            egress_proxy_profiles: vec![],
            egress_proxy_no_proxy: "localhost,.svc.cluster.local".into(),
        },
        proxy: ProxyConfig {
            domain: None,
            protocol: "http".into(),
            traefik_auth_middleware_name: "test".into(),
            traefik_auth_middleware_namespace: "test".into(),
            traefik_entry_point: "web".into(),
            traefik_ingress_namespaces: vec!["test".into()],
        },
        docker: DockerConfig {
            network: "moatless".into(),
            image: "sandbox:latest".into(),
            data_dir: "/data".into(),
            container_data_dir: "/data".into(),
            backend_hostname: "moatless-backend".into(),
            enable_socket_mount: false,
            socket_path: "/var/run/docker.sock".into(),
            extra_networks: vec![],
        },
        database_url: None,
        database: DatabaseConfig {
            url: None,
            max_connections: 5,
            run_migrations: true,
        },
        redis: RedisConfig {
            url: None,
            max_stream_length: 100,
            event_handler_enabled: true,
        },
        sse: SseConfig {
            broadcast_capacity: 16,
            consumer_id: "test".into(),
        },
        auth: AuthConfig {
            provider: moatless_types::auth::AuthProvider::Local,
            github_required_org: None,
            auth0_domain: None,
            auth0_client_id: None,
            auth0_client_secret: None,
            auth0_callback_url: None,
            auth0_audience: None,
            auth0_groups_claim: String::new(),
            oidc: None,
            default_team_slug: None,
            session_max_age_secs: 3600,
            session_absolute_age_secs: 3600,
            frontend_url: "http://localhost:3000".into(),
            csrf_trusted_origins: Vec::new(),
            cookie_domain: None,
            session_cookie_name: "test".into(),
            is_production: false,
            admin_username: "admin".into(),
            admin_email: Some("admin@example.com".into()),
            admin_password: None,
            break_glass_enabled: false,
            cache_ttl_secs: 0,
            session_jwt_secret: None,
            test_api_key: None,
            github_app_client_id: None,
            github_app_client_secret: None,
            github_app_callback_url: None,
            github_app_id: None,
            github_app_private_key: None,
            github_app_default_installation_id: None,
            github_apps: vec![],
        },
        s3: S3Config {
            bucket_name: None,
            key_prefix: "test".into(),
            transcript_prefix: "transcripts".into(),
            workspace_prefix: "workspaces".into(),
            region: "us-east-1".into(),
        },
        slack: SlackConfig {
            bot_token: None,
            signing_secret: None,
            bot_user_id: None,
        },
        telegram: TelegramConfig {
            bot_token: None,
            mode: "webhook".into(),
        },
        github_pr: moatless_core::config::GithubPrConfig {
            webhook_secret: None,
        },
        linear: moatless_types::adapter::LinearConfig {
            webhook_secret: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            oauth_callback_url: None,
            api_base_url: None,
        },
        skill: moatless_core::config::SkillConfig { cache_ttl_secs: 60 },
        secrets: moatless_types::secret::SecretsConfig {
            encryption_key: Some("0123456789abcdef0123456789abcdef".to_string()),
            kms_key_id: None,
        },
        codex_oauth: moatless_types::agent::CodexOAuthConfig {
            issuer: "https://auth.openai.com".into(),
            client_id: "test-codex-client".into(),
        },
        otel: OtelConfig {
            otlp_endpoint: None,
            sampling_ratio: 0.1,
            service_name: "test".into(),
        },
    }
}

pub(super) fn test_plan() -> ProvisionPlan {
    ProvisionPlan {
        repo_config: RepoConfig {
            name: "repo".into(),
            docker_image: None,
            servers: Some(vec![ServerConfig {
                name: "web".into(),
                port: Port::new(3000),
                start_commands: vec!["bun run dev".into()],
                ..Default::default()
            }]),
            system_prompt: None,
            code_execution: None,
            preview_config: None,
            resources: None,
            plugins: None,
            docker_access: None,
            docker_networks: None,
            sandbox_service_account: None,
            network_profile: None,
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
