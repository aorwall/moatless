//! Build the local sandbox process's environment from a `ProvisionPlan`.
//!
//! The list follows `moatless-sandbox-docker`'s `container_builder.rs`, which
//! this crate may not import (same band), then points every path and port at
//! the task's own, since sibling sandboxes share one host.

use moatless_core::config::Config;
use moatless_sandbox_runtime::provisioning::ProvisionPlan;
use moatless_workspace_types::workspace::WORKSPACE_REPOS_ENV;

use super::naming::TaskPaths;

/// Env vars the backend's own process may hold credentials/settings under,
/// passed through to the sandbox process only when the plan doesn't already
/// supply them.
const PASSTHROUGH_ENV_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "CODEX_AUTH_JSON",
    "PATH",
];

#[allow(clippy::too_many_arguments)]
pub fn build_env(
    config: &Config,
    task_id: &str,
    plan: &ProvisionPlan,
    user_auth_token: Option<&str>,
    paths: &TaskPaths,
    http_port: u16,
    grpc_port: u16,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = vec![
        ("TASK_ID".to_string(), task_id.to_string()),
        ("SANDBOX_ID".to_string(), task_id.to_string()),
        ("GIT_CLONE_URL".to_string(), plan.git_clone_url.clone()),
        ("GIT_PROVIDER".to_string(), plan.git_provider.clone()),
    ];

    if let Some(ref branch) = plan.git_branch {
        env.push(("GIT_BRANCH".to_string(), branch.clone()));
    }

    if !plan.repos.is_empty() {
        match serde_json::to_string(&plan.repos) {
            Ok(json) => env.push((WORKSPACE_REPOS_ENV.to_string(), json)),
            Err(e) => tracing::warn!(
                error = %e,
                "Failed to serialize WORKSPACE_REPOS; sandbox will clone the primary repo only"
            ),
        }
    }

    if let Some(token) = user_auth_token {
        env.push(("USER_AUTH_TOKEN".to_string(), token.to_string()));
    }

    if let Some(repo_config_json) =
        moatless_sandbox_runtime::repo_config_encoding::encode_repo_config(&plan.repo_config)
    {
        env.push(("REPO_CONFIG".to_string(), repo_config_json));
    }

    if let Some(ref domain) = config.proxy.domain {
        env.push(("PROXY_DOMAIN".to_string(), domain.clone()));
        env.push(("PROXY_PROTOCOL".to_string(), config.proxy.protocol.clone()));
    }

    env.push(("FRONTEND_URL".to_string(), config.auth.frontend_url.clone()));

    if let Some(ref model) = config.sandbox.default_model {
        env.push(("DEFAULT_CLAUDE_CODE_MODEL".to_string(), model.clone()));
    }
    if let Some(ref model) = config.sandbox.default_codex_model {
        env.push(("DEFAULT_CODEX_MODEL".to_string(), model.clone()));
    }

    if let Some(ref env_vars) = plan.env_vars {
        for (key, value) in env_vars {
            env.push((key.clone(), value.clone()));
        }
    }

    // ── Local-process overrides ─────────────────────────────────────
    // The backend and every task's sandbox run as siblings on 127.0.0.1, so
    // there is no container-network hostname to resolve — everything talks to
    // 127.0.0.1 on a per-task port instead.
    env.push((
        "BACKEND_GRPC_URL".to_string(),
        format!("http://127.0.0.1:{}", config.server.grpc_port),
    ));
    env.push((
        "BACKEND_API_URL".to_string(),
        format!("http://127.0.0.1:{}", config.server.port),
    ));
    env.push(("PORT".to_string(), http_port.to_string()));
    env.push(("GRPC_PORT".to_string(), grpc_port.to_string()));

    env.push(("DATA_DIR".to_string(), paths.data_dir.display().to_string()));
    env.push((
        "WORKSPACE_DIR".to_string(),
        paths.workspace_dir.display().to_string(),
    ));
    // Mirrors `default_upload_dir()`'s own convention in the sandbox binary
    // (`{WORKSPACE_DIR}/uploads`) rather than Docker's `file_mount_path`, which
    // names a container-internal mount path with no meaning on the shared
    // host filesystem.
    env.push((
        "UPLOAD_DIR".to_string(),
        paths.upload_dir().display().to_string(),
    ));
    env.push(("HOME".to_string(), paths.home_dir.display().to_string()));
    env.push((
        "CLAUDE_CONFIG_DIR".to_string(),
        paths.claude_config_dir.display().to_string(),
    ));
    env.push((
        "CODEX_HOME".to_string(),
        paths.codex_home_dir().display().to_string(),
    ));
    env.push(("TMPDIR".to_string(), paths.tmp_dir.display().to_string()));
    env.push((
        "RUNTIME_DIR".to_string(),
        paths.runtime_dir.display().to_string(),
    ));

    for key in PASSTHROUGH_ENV_VARS {
        if env.iter().any(|(k, _)| k == key) {
            continue;
        }
        if let Ok(value) = std::env::var(key) {
            env.push(((*key).to_string(), value));
        }
    }

    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_sandbox_runtime::provisioning::RepoConfig;

    fn test_config() -> Config {
        use moatless_types::sandbox::SandboxProviderType;
        let mut config = Config::test_minimal();
        config.sandbox_provider = SandboxProviderType::Local;
        config
    }

    fn test_plan() -> ProvisionPlan {
        ProvisionPlan {
            repo_config: RepoConfig {
                name: "test-repo".into(),
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
            git_branch: Some("main".into()),
            git_provider: "github".into(),
            env_vars: None,
            repos: Vec::new(),
        }
    }

    #[test]
    fn env_includes_required_vars_and_local_overrides() {
        let config = test_config();
        let plan = test_plan();
        let paths = super::super::naming::task_paths("/root", "task-1").unwrap();
        let env = build_env(
            &config,
            "task-1",
            &plan,
            Some("jwt-token"),
            &paths,
            9001,
            9002,
        );

        let get = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        assert_eq!(get("TASK_ID"), Some("task-1"));
        assert_eq!(get("SANDBOX_ID"), Some("task-1"));
        assert_eq!(get("GIT_BRANCH"), Some("main"));
        assert_eq!(get("USER_AUTH_TOKEN"), Some("jwt-token"));
        assert_eq!(get("PORT"), Some("9001"));
        assert_eq!(get("GRPC_PORT"), Some("9002"));
        assert_eq!(
            get("BACKEND_GRPC_URL"),
            Some(format!("http://127.0.0.1:{}", config.server.grpc_port)).as_deref()
        );
        assert_eq!(get("WORKSPACE_DIR"), Some("/root/tasks/task-1/workspace"));
        assert_eq!(
            get("UPLOAD_DIR"),
            Some("/root/tasks/task-1/workspace/uploads")
        );
        assert_eq!(get("HOME"), Some("/root/tasks/task-1/home"));
        assert_eq!(get("CODEX_HOME"), Some("/root/tasks/task-1/home/.codex"));
        assert_eq!(get("RUNTIME_DIR"), Some("/root/tasks/task-1/runtime"));
    }

    #[test]
    fn plan_env_vars_take_precedence_over_passthrough() {
        let config = test_config();
        let mut plan = test_plan();
        plan.env_vars = Some(std::collections::HashMap::from([(
            "PATH".to_string(),
            "/custom/path".to_string(),
        )]));
        let paths = super::super::naming::task_paths("/root", "task-1").unwrap();
        let env = build_env(&config, "task-1", &plan, None, &paths, 1, 2);
        let path_values: Vec<&str> = env
            .iter()
            .filter(|(k, _)| k == "PATH")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(path_values, vec!["/custom/path"]);
    }
}
