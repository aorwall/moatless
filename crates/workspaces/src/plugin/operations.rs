//! The plugin capability a host holds.

use std::sync::Arc;

use moatless_core::config::DeclaredPlugin;
use tracing::warn;

use super::dao::PluginDao;

/// The plugin, skill and activation tables, as a host and this crate's own
/// routes reach them.
///
/// The store stays private to this crate; a peer band resolving what a Sandbox
/// delivers takes
/// [`moatless_workspace_ports::WorkspaceResolver::plugin_deliveries`] instead.
pub struct Plugins {
    store: Arc<dyn PluginDao>,
}

impl Plugins {
    pub(crate) fn new(store: Arc<dyn PluginDao>) -> Self {
        Self { store }
    }

    /// Reconcile the deployment's Plugins; see [`PluginDao::reconcile_declared`].
    /// Writes no activation, fetches no git, and only logs a failure.
    pub async fn reconcile_declared(&self, declared: &[DeclaredPlugin]) {
        match self.store.reconcile_declared(declared).await {
            Ok(shadowed) => {
                for name in shadowed {
                    warn!(plugin = %name, "Declared plugin name is held by an admin-registered plugin; leaving it alone");
                }
            }
            Err(e) => warn!(error = %e, "Could not reconcile declared plugins"),
        }
    }

    /// The store, for this crate's own handlers, registry and sync service.
    pub(crate) fn store(&self) -> &dyn PluginDao {
        self.store.as_ref()
    }
}
