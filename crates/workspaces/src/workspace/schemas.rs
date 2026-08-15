//! Workspace request/response types.
//!
//! Re-exported from the `moatless-api-schemas` crate, with local `From`
//! conversions from the DB row models.

pub use moatless_api_schemas::workspace::*;

use crate::repository::schemas::scope_to_wire;

use super::models::{
    ResourceConfig as DomainResourceConfig, ScriptConfig as DomainScriptConfig,
    ServerConfig as DomainServerConfig, Workspace,
    WorkspaceCredentialProvider as DomainCredentialProvider, WorkspaceRepo,
};

pub fn credential_provider_from_wire(
    provider: WorkspaceCredentialProvider,
) -> DomainCredentialProvider {
    match provider {
        WorkspaceCredentialProvider::Github => DomainCredentialProvider::Github,
        WorkspaceCredentialProvider::Gitlab => DomainCredentialProvider::Gitlab,
        WorkspaceCredentialProvider::Gitness => DomainCredentialProvider::Gitness,
        WorkspaceCredentialProvider::Openai => DomainCredentialProvider::Openai,
        WorkspaceCredentialProvider::Anthropic => DomainCredentialProvider::Anthropic,
    }
}

pub fn credential_provider_to_wire(
    provider: DomainCredentialProvider,
) -> WorkspaceCredentialProvider {
    match provider {
        DomainCredentialProvider::Github => WorkspaceCredentialProvider::Github,
        DomainCredentialProvider::Gitlab => WorkspaceCredentialProvider::Gitlab,
        DomainCredentialProvider::Gitness => WorkspaceCredentialProvider::Gitness,
        DomainCredentialProvider::Openai => WorkspaceCredentialProvider::Openai,
        DomainCredentialProvider::Anthropic => WorkspaceCredentialProvider::Anthropic,
    }
}

impl From<ResourceConfig> for DomainResourceConfig {
    fn from(config: ResourceConfig) -> Self {
        Self {
            requests: config.requests,
            limits: config.limits,
            storage: config.storage,
            build_cache: config.build_cache,
            build_cache_mode: config.build_cache_mode,
            build_cache_scope: config.build_cache_scope,
            pvc_enabled: config.pvc_enabled,
            s3_enabled: config.s3_enabled,
            docker_storage: config.docker_storage,
            docker_storage_mode: config.docker_storage_mode,
        }
    }
}

impl From<DomainResourceConfig> for ResourceConfig {
    fn from(config: DomainResourceConfig) -> Self {
        Self {
            requests: config.requests,
            limits: config.limits,
            storage: config.storage,
            build_cache: config.build_cache,
            build_cache_mode: config.build_cache_mode,
            build_cache_scope: config.build_cache_scope,
            pvc_enabled: config.pvc_enabled,
            s3_enabled: config.s3_enabled,
            docker_storage: config.docker_storage,
            docker_storage_mode: config.docker_storage_mode,
        }
    }
}

impl From<ServerConfig> for DomainServerConfig {
    fn from(config: ServerConfig) -> Self {
        Self {
            name: config.name,
            label: config.label,
            port: config.port,
            start_commands: config.start_commands,
            startup_timeout_seconds: config.startup_timeout_seconds,
            auto_reload: config.auto_reload,
            enable_inspector: config.enable_inspector,
            default: config.default,
            docker_image: config.docker_image,
            env_vars: config.env_vars,
            resources: config.resources.map(Into::into),
        }
    }
}

impl From<DomainServerConfig> for ServerConfig {
    fn from(config: DomainServerConfig) -> Self {
        Self {
            name: config.name,
            label: config.label,
            port: config.port,
            start_commands: config.start_commands,
            startup_timeout_seconds: config.startup_timeout_seconds,
            auto_reload: config.auto_reload,
            enable_inspector: config.enable_inspector,
            default: config.default,
            docker_image: config.docker_image,
            env_vars: config.env_vars,
            resources: config.resources.map(Into::into),
        }
    }
}

impl From<ScriptConfig> for DomainScriptConfig {
    fn from(config: ScriptConfig) -> Self {
        Self {
            name: config.name,
            label: config.label,
            command: config.command,
            icon: config.icon,
            port: config.port,
            env_vars: config.env_vars,
        }
    }
}

impl From<DomainScriptConfig> for ScriptConfig {
    fn from(config: DomainScriptConfig) -> Self {
        Self {
            name: config.name,
            label: config.label,
            command: config.command,
            icon: config.icon,
            port: config.port,
            env_vars: config.env_vars,
        }
    }
}

pub(super) fn server_configs_from_wire(configs: Vec<ServerConfig>) -> Vec<DomainServerConfig> {
    configs.into_iter().map(Into::into).collect()
}

pub(super) fn script_configs_from_wire(configs: Vec<ScriptConfig>) -> Vec<DomainScriptConfig> {
    configs.into_iter().map(Into::into).collect()
}

pub(super) fn resource_config_from_wire(config: ResourceConfig) -> DomainResourceConfig {
    config.into()
}

impl From<WorkspaceRepo> for WorkspaceRepoResponse {
    fn from(r: WorkspaceRepo) -> Self {
        Self {
            id: r.id,
            workspace_id: r.workspace_id,
            repository_id: r.repository_id,
            branch: r.branch,
            mount_name: r.mount_name,
            is_primary: r.is_primary,
            position: r.position,
            created_at: r.created_at.map(|t| t.to_rfc3339()),
        }
    }
}

/// Build a [`WorkspaceResponse`] from a workspace row plus its placements.
/// (A free function rather than `From<(..)>` — the orphan rule forbids
/// implementing a foreign trait for a foreign type keyed on a tuple.)
pub fn workspace_response(w: Workspace, repos: Vec<WorkspaceRepo>) -> WorkspaceResponse {
    WorkspaceResponse {
        id: w.id,
        name: w.name,
        description: w.description,
        kind: w.kind,
        docker_image: w.docker_image,
        docker_access: w.docker_access,
        sandbox_service_account: w.sandbox_service_account,
        system_prompt: w.system_prompt,
        setup_commands: w
            .setup_commands
            .and_then(|v| serde_json::from_value(v).ok()),
        env_vars: w.env_vars.and_then(|v| serde_json::from_value(v).ok()),
        servers: w
            .servers
            .and_then(|value| serde_json::from_value::<Vec<DomainServerConfig>>(value).ok())
            .map(|configs| configs.into_iter().map(Into::into).collect()),
        scripts: w
            .scripts
            .and_then(|value| serde_json::from_value::<Vec<DomainScriptConfig>>(value).ok())
            .map(|configs| configs.into_iter().map(Into::into).collect()),
        resources: w
            .resources
            .and_then(|value| serde_json::from_value::<DomainResourceConfig>(value).ok())
            .map(Into::into),
        network_profile: w.network_profile,
        synced_from_git: w.source == "git",
        source_config_path: w.source_config_path,
        source: w.source,
        source_repository_id: w.source_repository_id,
        source_key: w.source_key,
        scope: scope_to_wire(w.scope),
        owner_user_id: w.owner_user_id,
        created_by: w.created_by,
        deleted: w.deleted.unwrap_or(false),
        created_at: w.created_at.map(|t| t.to_rfc3339()),
        updated_at: w.updated_at.map(|t| t.to_rfc3339()),
        repos: repos.into_iter().map(Into::into).collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn every_credential_provider_maps_both_ways_with_identical_json() {
        for domain in DomainCredentialProvider::ALL {
            let wire = credential_provider_to_wire(*domain);
            assert_eq!(credential_provider_from_wire(wire), *domain);
            assert_eq!(
                serde_json::to_value(domain).expect("serialize"),
                serde_json::to_value(wire).expect("serialize")
            );
        }
    }

    #[test]
    fn workspace_run_config_maps_both_ways_without_changing_json() {
        let resources = DomainResourceConfig {
            requests: Some(HashMap::from([("cpu".to_string(), "500m".to_string())])),
            limits: Some(HashMap::from([("memory".to_string(), "2Gi".to_string())])),
            storage: Some("8Gi".to_string()),
            build_cache: Some("40Gi".to_string()),
            build_cache_mode: Some("node".to_string()),
            build_cache_scope: Some("task".to_string()),
            pvc_enabled: Some(true),
            s3_enabled: Some(false),
            docker_storage: Some("50Gi".to_string()),
            docker_storage_mode: Some("emptyDir".to_string()),
        };
        let wire_resources: ResourceConfig = resources.clone().into();
        assert_eq!(
            serde_json::to_value(&resources).expect("serialize"),
            serde_json::to_value(&wire_resources).expect("serialize")
        );
        assert_eq!(DomainResourceConfig::from(wire_resources), resources);

        let server = DomainServerConfig {
            name: "web".to_string(),
            label: Some("Web".to_string()),
            port: 3000,
            start_commands: Some(vec!["bun run dev".to_string()]),
            startup_timeout_seconds: Some(90),
            auto_reload: true,
            enable_inspector: true,
            default: true,
            docker_image: Some("node:24".to_string()),
            env_vars: Some(HashMap::from([("MODE".to_string(), "dev".to_string())])),
            resources: Some(resources.clone()),
        };
        let wire_server: ServerConfig = server.clone().into();
        assert_eq!(
            serde_json::to_value(&server).expect("serialize"),
            serde_json::to_value(&wire_server).expect("serialize")
        );
        assert_eq!(DomainServerConfig::from(wire_server), server);

        let script = DomainScriptConfig {
            name: "seed".to_string(),
            label: Some("Seed".to_string()),
            command: "bun run seed".to_string(),
            icon: Some("database".to_string()),
            port: Some(4321),
            env_vars: Some(HashMap::from([("RESET".to_string(), "1".to_string())])),
        };
        let wire_script: ScriptConfig = script.clone().into();
        assert_eq!(
            serde_json::to_value(&script).expect("serialize"),
            serde_json::to_value(&wire_script).expect("serialize")
        );
        assert_eq!(DomainScriptConfig::from(wire_script), script);
    }
}
