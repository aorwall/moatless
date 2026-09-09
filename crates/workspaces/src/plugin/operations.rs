//! The plugin capability a host holds.

use std::sync::Arc;

use super::dao::PluginDao;

/// The plugin, skill and activation tables, as a host and this crate's own
/// routes reach them.
///
/// The store behind it stays private to this crate. A peer band that resolves
/// what a Sandbox delivers takes
/// [`moatless_ports::workspace::WorkspaceResolver::plugin_deliveries`] instead.
pub struct Plugins {
    store: Arc<dyn PluginDao>,
}

impl Plugins {
    pub(crate) fn new(store: Arc<dyn PluginDao>) -> Self {
        Self { store }
    }

    /// The store, for this crate's own handlers, registry and sync service.
    pub(crate) fn store(&self) -> &dyn PluginDao {
        self.store.as_ref()
    }
}
