//! Runtime provider and workspace archive assembly.

use crate::{compose::Ports, sandbox, startup};
use moatless_core::config::Config;
use moatless_types::sandbox::SandboxProviderType;
use std::sync::Arc;
use tracing::info;

/// Runtime capabilities built from one configured provider.
pub(crate) struct SandboxDependencies {
    pub workspace_archive: Option<Arc<sandbox::lifecycle::S3WorkspaceArchive>>,
    pub workspace_snapshots: Option<Arc<dyn moatless_ports::sandbox::WorkspaceSnapshotSource>>,
    pub provision_deps: Option<sandbox::lifecycle::ProvisionDeps>,
    pub sandbox_runtime: Arc<dyn moatless_sandbox_runtime::runtime::SandboxRuntime>,
    pub sandbox_observer: Arc<dyn sandbox::status_reader::SandboxStatusReader>,
    pub sandbox_connector: Arc<dyn sandbox_proto::client::SandboxConnector>,
    pub cluster_diagnostics: Arc<dyn moatless_ports::sandbox::ClusterDiagnostics>,
    pub sandbox_port_publisher: Arc<dyn moatless_ports::sandbox::SandboxPortPublisher>,
    pub default_sandbox_image: String,
    pub reaper_k8s: Option<sandbox::k8s::client::K8sClient>,
    pub docker_watcher_client: Option<sandbox::docker::client::DockerClient>,
    pub sandbox_fleet_observer: Arc<dyn moatless_sandbox_runtime::fleet::FleetObserver>,
    pub storage_fallback: Option<Arc<dyn sandbox::k8s::provider::StorageFallback>>,
}

pub(crate) async fn build(
    config: &Config,
    ports: Option<&Ports>,
    s3_client: Option<moatless_s3::S3Storage>,
) -> Result<SandboxDependencies, Box<dyn std::error::Error>> {
    let workspace_archive =
        s3_client
            .clone()
            .zip(config.s3.bucket_name.clone())
            .map(|(s3, bucket)| {
                Arc::new(crate::sandbox::lifecycle::S3WorkspaceArchive::new(
                    Arc::new(s3),
                    bucket,
                    config.s3.workspace_prefix.clone(),
                ))
            });
    // Reads a stopped Task's files out of what S3 holds, for the file endpoints
    // and the T3 read surfaces. Needs both S3 and the master secret that opens a
    // task's `restic` repository: without either there is no snapshot to read.
    let workspace_snapshots: Option<Arc<dyn moatless_ports::sandbox::WorkspaceSnapshotSource>> =
        s3_client
            .clone()
            .zip(config.s3.bucket_name.clone())
            .zip(config.sandbox.workspace_repository_secret.clone())
            .map(|((s3, bucket), secret)| {
                Arc::new(
                    crate::sandbox::workspace_snapshot::ResticWorkspaceSnapshots::new(
                        s3.client.clone(),
                        bucket,
                        config.s3.workspace_prefix.clone(),
                        secret,
                    ),
                ) as Arc<dyn moatless_ports::sandbox::WorkspaceSnapshotSource>
            });
    let provision_deps = ports.map(|p| {
        p.sandbox_graph
            .provision_deps(p.workspace_resolver.clone(), p.user_directory.clone())
    });
    // Build the configured runtime/observer/connector adapter once. The rich
    // 13-field read the HTTP routes need is the backend-local
    // `SandboxStatusReader`; the T3 contract observer is projected from it at
    // its one construction site, and the one connector serves both surfaces.
    let sandbox_runtime: Arc<dyn moatless_sandbox_runtime::runtime::SandboxRuntime>;
    let sandbox_observer: Arc<dyn sandbox::status_reader::SandboxStatusReader>;
    let sandbox_connector: Arc<dyn sandbox_proto::client::SandboxConnector>;
    let cluster_diagnostics: Arc<dyn moatless_ports::sandbox::ClusterDiagnostics>;
    let sandbox_port_publisher: Arc<dyn moatless_ports::sandbox::SandboxPortPublisher>;
    let default_sandbox_image: String;
    let reaper_k8s: Option<sandbox::k8s::client::K8sClient>;
    let docker_watcher_client: Option<sandbox::docker::client::DockerClient>;
    // The reconciler's per-tick fleet observation, built per runtime backend.
    let sandbox_fleet_observer: Arc<dyn moatless_sandbox_runtime::fleet::FleetObserver>;
    // Attach-failure remediation for the pod watcher — K8s only; Docker has no
    // PVC to fail to attach.
    let storage_fallback: Option<Arc<dyn sandbox::k8s::provider::StorageFallback>>;

    match config.sandbox_provider {
        SandboxProviderType::Kubernetes => {
            let k8s =
                sandbox::k8s::client::K8sClient::new(config.sandbox.namespace.clone()).await?;
            info!("K8s client initialized");

            reaper_k8s = Some(k8s.clone());
            docker_watcher_client = None;
            let adapter = startup::build_k8s_sandbox_adapter(
                config,
                k8s.clone(),
                s3_client
                    .clone()
                    .map(|storage| std::sync::Arc::new(storage.client)),
                provision_deps.clone(),
            );
            sandbox_runtime = adapter.clone();
            sandbox_observer = adapter.clone();
            sandbox_connector = adapter.clone();
            sandbox_port_publisher = adapter.clone();
            storage_fallback = Some(adapter.clone());
            cluster_diagnostics = adapter;
            default_sandbox_image = config.sandbox.image.clone();
            sandbox_fleet_observer = Arc::new(sandbox::k8s::fleet_observer::K8sFleetObserver::new(
                k8s.clone(),
                sandbox_observer.clone(),
            ));

            // The K8s pod lifecycle watcher is spawned after the DB repos are
            // built (it needs them to auto-disable crash-looping servers).
        }
        SandboxProviderType::Docker => {
            let docker_client = sandbox::docker::client::DockerClient::new(
                config.docker.network.clone(),
                config.docker.data_dir.clone(),
                config.docker.container_data_dir.clone(),
            )?;
            info!("Docker client initialized");

            reaper_k8s = None;
            docker_watcher_client = Some(docker_client.clone());
            storage_fallback = None;
            let adapter = startup::build_docker_sandbox_adapter(
                config,
                docker_client.clone(),
                s3_client
                    .clone()
                    .map(|storage| std::sync::Arc::new(storage.client)),
                provision_deps.clone(),
            );
            sandbox_runtime = adapter.clone();
            sandbox_observer = adapter.clone();
            sandbox_connector = adapter.clone();
            sandbox_port_publisher = adapter.clone();
            cluster_diagnostics = adapter;
            default_sandbox_image = config.docker.image.clone();
            sandbox_fleet_observer =
                Arc::new(sandbox::docker::fleet_observer::DockerFleetObserver::new(
                    docker_client.clone(),
                    sandbox_observer.clone(),
                ));

            // The Docker watcher is spawned after lifecycle is assembled so
            // every observation is classified against durable intent.
        }
    }

    Ok(SandboxDependencies {
        workspace_archive,
        workspace_snapshots,
        provision_deps,
        sandbox_runtime,
        sandbox_observer,
        sandbox_connector,
        cluster_diagnostics,
        sandbox_port_publisher,
        default_sandbox_image,
        reaper_k8s,
        docker_watcher_client,
        sandbox_fleet_observer,
        storage_fallback,
    })
}

pub(crate) async fn lifecycle(
    config: &Config,
    ports: Option<&Ports>,
    dependencies: &SandboxDependencies,
    events: Option<Arc<dyn moatless_core::events::EventPublisher>>,
) -> Result<Option<Arc<sandbox::lifecycle::Lifecycle>>, Box<dyn std::error::Error>> {
    Ok(match ports {
        Some(ports) => {
            let db_url = config
                .database_url
                .as_deref()
                .ok_or("DATABASE_URL required to build the lifecycle lock pool")?;
            let lock_pool = moatless_core::db::create_pool_kind(
                db_url,
                config.database.max_connections,
                moatless_core::db::PoolKind::Lock,
            )
            .await?;
            Some(
                ports
                    .sandbox_graph
                    .lifecycle(moatless_sandbox_control::compose::LifecycleInputs {
                        lock_pool,
                        runtime: dependencies.sandbox_runtime.clone(),
                        connector: dependencies.sandbox_connector.clone(),
                        plan: dependencies
                            .provision_deps
                            .clone()
                            .ok_or("provision dependencies missing with database pool")?,
                        default_sandbox_image: dependencies.default_sandbox_image.clone(),
                        events,
                        workspace_archive: dependencies.workspace_archive.clone(),
                    }),
            )
        }
        None => None,
    })
}
