//! Docker client wrapper around bollard.

use std::sync::Arc;

use bollard::Docker;

/// Shared Docker client state.
///
/// Cheap to clone — `bollard::Docker` uses an internal Arc'd HTTP client,
/// and configuration fields are wrapped in `Arc`.
#[derive(Clone)]
pub struct DockerClient {
    /// The underlying Docker API client.
    pub docker: Docker,
    /// Docker network name for sandbox containers.
    pub network: Arc<String>,
    /// Host path for shared data volumes (must be absolute host path for
    /// sibling container bind mounts).
    pub data_dir: Arc<String>,
    /// Container-internal path where data is mounted.
    pub container_data_dir: Arc<String>,
}

impl DockerClient {
    /// Create a new Docker client connecting via the default Unix socket.
    ///
    /// On Linux/macOS this connects to `/var/run/docker.sock`.
    pub fn new(
        network: String,
        data_dir: String,
        container_data_dir: String,
    ) -> Result<Self, bollard::errors::Error> {
        let docker = Docker::connect_with_socket_defaults()?;
        Ok(Self {
            docker,
            network: Arc::new(network),
            data_dir: Arc::new(data_dir),
            container_data_dir: Arc::new(container_data_dir),
        })
    }
}
