//! Workspace request/response types.
//!
//! Re-exported from the `moatless-api-schemas` crate, with local `From`
//! conversions from the DB row models.

pub use moatless_api_schemas::workspace::*;

use crate::repository::schemas::scope_to_wire;

use moatless_backend_core::run_config::{
    Cpu, EnvKey, EnvVars, Memory, Port, ResourceConfig as DomainResourceConfig,
    ResourceQuantities as DomainResourceQuantities, ServerConfig as DomainServerConfig,
    StorageMedium as DomainStorageMedium,
};
use tracing::error;

use super::error::WorkspaceError;

use super::models::{
    ScriptConfig as DomainScriptConfig, Workspace,
    WorkspaceCredentialProvider as DomainCredentialProvider, WorkspaceRepo,
};

pub fn credential_provider_from_wire(
    provider: WorkspaceCredentialProvider,
) -> DomainCredentialProvider {
    match provider {
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
        DomainCredentialProvider::Gitlab => WorkspaceCredentialProvider::Gitlab,
        DomainCredentialProvider::Gitness => WorkspaceCredentialProvider::Gitness,
        DomainCredentialProvider::Openai => WorkspaceCredentialProvider::Openai,
        DomainCredentialProvider::Anthropic => WorkspaceCredentialProvider::Anthropic,
    }
}

pub fn storage_medium_from_wire(medium: StorageMedium) -> DomainStorageMedium {
    match medium {
        StorageMedium::EmptyDir => DomainStorageMedium::EmptyDir,
        StorageMedium::Node => DomainStorageMedium::Node,
    }
}

pub fn storage_medium_to_wire(medium: DomainStorageMedium) -> StorageMedium {
    match medium {
        DomainStorageMedium::EmptyDir => StorageMedium::EmptyDir,
        DomainStorageMedium::Node => StorageMedium::Node,
    }
}

/// Parse one wire quantity, naming the field in the rejection message.
fn parse_quantity<T: std::str::FromStr>(
    raw: Option<String>,
    field: &str,
) -> Result<Option<T>, WorkspaceError>
where
    T::Err: std::fmt::Display,
{
    raw.map(|value| {
        value
            .parse()
            .map_err(|e| WorkspaceError::BadRequest(format!("resources.{field}: {e}")))
    })
    .transpose()
}

fn quantities_from_wire(
    quantities: ResourceQuantities,
    direction: &str,
) -> Result<DomainResourceQuantities, WorkspaceError> {
    Ok(DomainResourceQuantities {
        cpu: parse_quantity::<Cpu>(quantities.cpu, &format!("{direction}.cpu"))?,
        memory: parse_quantity::<Memory>(quantities.memory, &format!("{direction}.memory"))?,
    })
}

fn quantities_to_wire(quantities: DomainResourceQuantities) -> ResourceQuantities {
    ResourceQuantities {
        cpu: quantities.cpu.map(|cpu| cpu.as_str().to_string()),
        memory: quantities.memory.map(|memory| memory.as_str().to_string()),
    }
}

/// Parse a wire resource block into the run-config domain type.
///
/// Both types are foreign to this crate, so the conversions are free functions
/// rather than `From`/`TryFrom` impls.
pub fn resource_config_from_wire(
    config: ResourceConfig,
) -> Result<DomainResourceConfig, WorkspaceError> {
    Ok(DomainResourceConfig {
        requests: quantities_from_wire(config.requests, "requests")?,
        limits: quantities_from_wire(config.limits, "limits")?,
        storage: parse_quantity::<Memory>(config.storage, "storage")?,
        pvc_enabled: config.pvc_enabled,
        s3_enabled: config.s3_enabled,
        docker_storage: parse_quantity::<Memory>(config.docker_storage, "dockerStorage")?,
        docker_storage_mode: config.docker_storage_mode.map(storage_medium_from_wire),
    })
}

pub fn resource_config_to_wire(config: DomainResourceConfig) -> ResourceConfig {
    ResourceConfig {
        requests: quantities_to_wire(config.requests),
        limits: quantities_to_wire(config.limits),
        storage: config.storage.map(|q| q.as_str().to_string()),
        pvc_enabled: config.pvc_enabled,
        s3_enabled: config.s3_enabled,
        docker_storage: config.docker_storage.map(|q| q.as_str().to_string()),
        docker_storage_mode: config.docker_storage_mode.map(storage_medium_to_wire),
    }
}

pub fn server_config_from_wire(config: ServerConfig) -> Result<DomainServerConfig, WorkspaceError> {
    let env_vars = config
        .env_vars
        .into_iter()
        .map(|(key, value)| {
            key.parse::<EnvKey>()
                .map(|key| (key, value))
                .map_err(WorkspaceError::BadRequest)
        })
        .collect::<Result<EnvVars, _>>()?;
    let port = config
        .port
        .map(|port| {
            Port::new(port).ok_or_else(|| {
                WorkspaceError::BadRequest(format!(
                    "Server '{}' declares port 0; omit the field to have one allocated",
                    config.name
                ))
            })
        })
        .transpose()?;
    Ok(DomainServerConfig {
        name: config.name,
        label: config.label,
        port,
        start_commands: config.start_commands,
        startup_timeout_seconds: config.startup_timeout_seconds,
        auto_reload: config.auto_reload,
        enable_inspector: config.enable_inspector,
        default: config.default,
        docker_image: config.docker_image.filter(|image| !image.is_empty()),
        env_vars,
        resources: config
            .resources
            .map(resource_config_from_wire)
            .transpose()?,
    })
}

pub fn server_config_to_wire(config: DomainServerConfig) -> ServerConfig {
    ServerConfig {
        name: config.name,
        label: config.label,
        port: config.port.map(Port::get),
        start_commands: config.start_commands,
        startup_timeout_seconds: config.startup_timeout_seconds,
        auto_reload: config.auto_reload,
        enable_inspector: config.enable_inspector,
        default: config.default,
        docker_image: config.docker_image,
        env_vars: config
            .env_vars
            .iter()
            .map(|(key, value)| (key.as_str().to_string(), value.clone()))
            .collect(),
        resources: config.resources.map(resource_config_to_wire),
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

pub(super) fn server_configs_from_wire(
    configs: Vec<ServerConfig>,
) -> Result<Vec<DomainServerConfig>, WorkspaceError> {
    configs.into_iter().map(server_config_from_wire).collect()
}

pub(super) fn script_configs_from_wire(configs: Vec<ScriptConfig>) -> Vec<DomainScriptConfig> {
    configs.into_iter().map(Into::into).collect()
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

/// Decode one JSONB run-config column, logging what was dropped.
///
/// A column that fails to decode reads as unset rather than failing the whole
/// response, so one bad value cannot make a workspace unreadable. The log is
/// what makes that recoverable: without it the field silently disappears.
fn decode_column<T: serde::de::DeserializeOwned>(
    value: serde_json::Value,
    workspace_id: &str,
    column: &str,
) -> Option<T> {
    match serde_json::from_value(value) {
        Ok(decoded) => Some(decoded),
        Err(e) => {
            error!(
                workspace_id,
                column,
                error = %e,
                "Failed to decode a workspace run-config column; reporting it as unset"
            );
            None
        }
    }
}

/// Build a [`WorkspaceResponse`] from a workspace row plus its placements.
/// (A free function rather than `From<(..)>` — the orphan rule forbids
/// implementing a foreign trait for a foreign type keyed on a tuple.)
pub fn workspace_response(w: Workspace, repos: Vec<WorkspaceRepo>) -> WorkspaceResponse {
    let workspace_id = w.id.clone();
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
            .and_then(|value| decode_column(value, &workspace_id, "servers"))
            .map(|configs: Vec<DomainServerConfig>| {
                configs.into_iter().map(server_config_to_wire).collect()
            }),
        scripts: w
            .scripts
            .and_then(|value| decode_column(value, &workspace_id, "scripts"))
            .map(|configs: Vec<DomainScriptConfig>| configs.into_iter().map(Into::into).collect()),
        resources: w
            .resources
            .and_then(|value| {
                decode_column::<DomainResourceConfig>(value, &workspace_id, "resources")
            })
            .map(resource_config_to_wire),
        network_profile: w.network_profile,
        synced_from_git: w.source == "git",
        source_config_path: w.source_config_path,
        source: w.source,
        source_repository_id: w.source_repository_id,
        source_key: w.source_key,
        scope: scope_to_wire(w.scope),
        owner_user_id: w.owner_user_id,
        run_as_user_id: w.run_as_user_id,
        created_by: w.created_by,
        deleted: w.deleted.unwrap_or(false),
        created_at: w.created_at.map(|t| t.to_rfc3339()),
        updated_at: w.updated_at.map(|t| t.to_rfc3339()),
        repos: repos.into_iter().map(Into::into).collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

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

    fn quantity<T: std::str::FromStr>(value: &str) -> T
    where
        T::Err: std::fmt::Debug,
    {
        value.parse().expect("parse quantity")
    }

    /// Every field of both run-config types, in both directions, with the JSON
    /// on each side compared. The domain types are parsed while the wire types
    /// are strings, so this is what keeps the pair from drifting.
    #[test]
    fn workspace_run_config_maps_both_ways_without_changing_json() {
        let resources = DomainResourceConfig {
            requests: DomainResourceQuantities {
                cpu: Some(quantity("500m")),
                memory: Some(quantity("1Gi")),
            },
            limits: DomainResourceQuantities {
                cpu: Some(quantity("4")),
                memory: Some(quantity("2Gi")),
            },
            storage: Some(quantity("8Gi")),
            pvc_enabled: Some(true),
            s3_enabled: Some(false),
            docker_storage: Some(quantity("50Gi")),
            docker_storage_mode: Some(DomainStorageMedium::EmptyDir),
        };
        let wire_resources = resource_config_to_wire(resources.clone());
        assert_eq!(
            serde_json::to_value(&resources).expect("serialize"),
            serde_json::to_value(&wire_resources).expect("serialize")
        );
        assert_eq!(
            resource_config_from_wire(wire_resources).expect("parse back"),
            resources
        );

        let server = DomainServerConfig {
            name: "web".to_string(),
            label: Some("Web".to_string()),
            port: Port::new(3000),
            start_commands: vec!["bun run dev".to_string()],
            startup_timeout_seconds: Some(90),
            auto_reload: true,
            enable_inspector: true,
            default: true,
            docker_image: Some("node:24".to_string()),
            env_vars: [(
                "MODE".parse::<EnvKey>().expect("env key"),
                "dev".to_string(),
            )]
            .into_iter()
            .collect(),
            resources: Some(resources.clone()),
        };
        let wire_server = server_config_to_wire(server.clone());
        assert_eq!(
            serde_json::to_value(&server).expect("serialize"),
            serde_json::to_value(&wire_server).expect("serialize")
        );
        assert_eq!(
            server_config_from_wire(wire_server).expect("parse back"),
            server
        );

        // The empty forms too: both sides must omit the same keys, or a
        // workspace with no servers round-trips into a different document.
        let empty = DomainServerConfig {
            name: "bare".to_string(),
            ..Default::default()
        };
        let wire_empty = server_config_to_wire(empty.clone());
        assert_eq!(
            serde_json::to_value(&empty).expect("serialize"),
            serde_json::to_value(&wire_empty).expect("serialize")
        );
        assert_eq!(
            server_config_from_wire(wire_empty).expect("parse back"),
            empty
        );

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

    #[test]
    fn every_storage_medium_maps_both_ways_with_identical_json() {
        for domain in DomainStorageMedium::ALL {
            let wire = storage_medium_to_wire(*domain);
            assert_eq!(storage_medium_from_wire(wire), *domain);
            assert_eq!(
                serde_json::to_value(domain).expect("serialize"),
                serde_json::to_value(wire).expect("serialize")
            );
        }
    }

    #[test]
    fn a_malformed_quantity_is_rejected_at_the_write_boundary() {
        let config = ResourceConfig {
            requests: ResourceQuantities {
                cpu: Some("lots".to_string()),
                memory: None,
            },
            ..Default::default()
        };
        let error = resource_config_from_wire(config).expect_err("must reject");
        assert!(
            error.to_string().contains("requests.cpu"),
            "the message must name the field: {error}"
        );
    }

    #[test]
    fn a_malformed_env_var_name_is_rejected_at_the_write_boundary() {
        let config = ServerConfig {
            name: "web".to_string(),
            env_vars: BTreeMap::from([("WITH-DASH".to_string(), "1".to_string())]),
            ..Default::default()
        };
        assert!(server_config_from_wire(config).is_err());
    }

    #[test]
    fn a_declared_port_of_zero_is_rejected_at_the_write_boundary() {
        let config = ServerConfig {
            name: "web".to_string(),
            port: Some(0),
            ..Default::default()
        };
        assert!(server_config_from_wire(config).is_err());
    }
}
