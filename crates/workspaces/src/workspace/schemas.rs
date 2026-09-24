//! Workspace request/response types.
//!
//! Re-exported from the `moatless-workspace-types::schemas` module, with local `From`
//! conversions from the DB row models.

pub use moatless_workspace_types::schemas::workspace::*;

use moatless_workspace_types::workspace::{
    Cpu, Memory, ResourceConfig as DomainResourceConfig,
    ResourceQuantities as DomainResourceQuantities,
};

use super::error::WorkspaceError;

use super::models::{Workspace, WorkspaceRepo, WorkspaceSource};
use moatless_workspace_types::workspace::ScriptConfig as DomainScriptConfig;

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
        docker_storage_mode: config.docker_storage_mode,
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
        docker_storage_mode: config.docker_storage_mode,
    }
}

pub(super) fn script_configs_from_wire(configs: Vec<ScriptConfig>) -> Vec<DomainScriptConfig> {
    configs
}

pub fn workspace_repo_response(r: WorkspaceRepo) -> WorkspaceRepoResponse {
    WorkspaceRepoResponse {
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

/// Build a [`WorkspaceResponse`] from a workspace row plus its placements.
///
/// A free function rather than `From<(..)>`: the orphan rule forbids
/// implementing a foreign trait for a foreign type keyed on a tuple.
pub fn workspace_response(w: Workspace, repos: Vec<WorkspaceRepo>) -> WorkspaceResponse {
    WorkspaceResponse {
        id: w.id,
        name: w.name,
        description: w.description,
        icon: w.icon,
        kind: w.kind.to_string(),
        docker_image: w.docker_image,
        docker_access: w.docker_access,
        sandbox_service_account: w.sandbox_service_account,
        system_prompt: w.system_prompt,
        setup_commands: w.setup_commands,
        env_vars: w.env_vars,
        scripts: w.scripts,
        resources: w.resources.map(resource_config_to_wire),
        network_profile: w.network_profile,
        node_selector: w.node_selector,
        default_model: w.default_model,
        default_effort: w.default_effort,
        synced_from_git: w.source == WorkspaceSource::Git,
        source_config_path: w.source_config_path,
        source: w.source.to_string(),
        source_repository_id: w.source_repository_id,
        source_key: w.source_key,
        scope: w.scope,
        owner_user_id: w.owner_user_id,
        run_as_user_id: w.run_as_user_id,
        created_by: w.created_by,
        deleted: w.deleted,
        created_at: w.created_at.map(|t| t.to_rfc3339()),
        updated_at: w.updated_at.map(|t| t.to_rfc3339()),
        repos: repos.into_iter().map(workspace_repo_response).collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

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
            docker_storage_mode: Some(StorageMedium::EmptyDir),
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

        let script = DomainScriptConfig {
            name: "seed".to_string(),
            label: Some("Seed".to_string()),
            command: "bun run seed".to_string(),
            icon: Some("database".to_string()),
            port: Some(4321),
            env_vars: Some(HashMap::from([("RESET".to_string(), "1".to_string())])),
            run_on_start: true,
        };
        assert_eq!(
            serde_json::to_value(&script).expect("serialize")["envVars"]["RESET"],
            "1"
        );
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
}
