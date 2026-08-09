//! Function-shaped registration helpers for typed Adapter delivery work.
//!
//! Provider modules still own their payloads, kind names, and runtime
//! functions. This module owns the repeated worker wrapper mechanics: decode
//! the stored JSON payload once, classify malformed rows as permanent, and, for
//! connection-scoped work, re-read the adapter connection in the system scope.
//!
//! The handler carries no application state: each provider closure captures the
//! values its own work needs at registration time, so this module names no
//! `AppState` and the only port it touches directly is the one
//! `register_connection_delivery` re-reads a connection through — passed in as
//! an explicit `AdapterConnectionDao`, not reached for off a bag.

use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

use moatless_backend_core::scope::ScopeContext;

use crate::adapters::dao::AdapterConnectionDao;
use crate::adapters::error::SharedAdapterError;
use crate::adapters::models::AdapterConnection;

use super::handlers::{AdapterDeliveryHandler, AdapterDeliveryHandlers};
use super::models::{AdapterDelivery, AdapterDeliveryFailure};

/// A provider-owned payload whose work is scoped to one adapter connection.
pub trait ConnectionDeliveryPayload: serde::de::DeserializeOwned + Send + Sync + 'static {
    fn connection_id(&self) -> &str;
}

/// Register a typed handler that receives the decoded provider payload.
///
/// The `execute` closure carries whatever it needs (captured at registration);
/// this helper adds only the payload decode and the handler wrapper.
pub fn register_typed_delivery<P, F, Fut>(
    handlers: &mut AdapterDeliveryHandlers,
    kind: &'static str,
    execute: F,
) where
    P: serde::de::DeserializeOwned + Send + Sync + 'static,
    F: Fn(AdapterDelivery, P) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), AdapterDeliveryFailure>> + Send + 'static,
{
    handlers.register(
        kind,
        Arc::new(TypedDeliveryHandler {
            kind,
            execute,
            _payload: PhantomData::<fn() -> P>,
        }),
    );
}

/// Register a typed handler for work that should re-read its connection before
/// running provider behavior.
///
/// The connection is re-read through the `adapter_connections` port passed here,
/// in the system scope — a durable delivery runs in the worker with no user
/// session.
pub fn register_connection_delivery<P, F, Fut>(
    handlers: &mut AdapterDeliveryHandlers,
    kind: &'static str,
    adapter_connections: Arc<dyn AdapterConnectionDao>,
    execute: F,
) where
    P: ConnectionDeliveryPayload,
    F: Fn(AdapterDelivery, AdapterConnection, P) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<(), AdapterDeliveryFailure>> + Send + 'static,
{
    register_typed_delivery::<P, _, _>(handlers, kind, move |delivery, payload| {
        let execute = execute.clone();
        let adapter_connections = adapter_connections.clone();
        async move {
            let connection =
                resolve_connection(adapter_connections.as_ref(), payload.connection_id()).await?;
            execute(delivery, connection, payload).await
        }
    });
}

struct TypedDeliveryHandler<P, F> {
    kind: &'static str,
    execute: F,
    _payload: PhantomData<fn() -> P>,
}

#[async_trait::async_trait]
impl<P, F, Fut> AdapterDeliveryHandler for TypedDeliveryHandler<P, F>
where
    P: serde::de::DeserializeOwned + Send + Sync + 'static,
    F: Fn(AdapterDelivery, P) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), AdapterDeliveryFailure>> + Send + 'static,
{
    async fn execute(&self, delivery: &AdapterDelivery) -> Result<(), AdapterDeliveryFailure> {
        let payload = serde_json::from_value(delivery.payload.clone()).map_err(|e| {
            AdapterDeliveryFailure::permanent(PayloadDecodeError {
                kind: self.kind,
                source: e,
            })
        })?;
        (self.execute)(delivery.clone(), payload).await
    }
}

async fn resolve_connection(
    adapter_connections: &dyn AdapterConnectionDao,
    connection_id: &str,
) -> Result<AdapterConnection, AdapterDeliveryFailure> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a durable Adapter delivery runs in the worker with no user session; the connection \
                  is named by an already-admitted record, not by caller input"
    )]
    let sys_ctx = ScopeContext::system();
    adapter_connections
        .find_by_id(connection_id, &sys_ctx)
        .await
        .map_err(|e| AdapterDeliveryFailure::retryable(SharedAdapterError::Db(e)))?
        .ok_or_else(|| {
            AdapterDeliveryFailure::permanent(ConnectionGone {
                connection_id: connection_id.to_string(),
            })
        })
}

#[derive(Debug, thiserror::Error)]
#[error("Adapter delivery payload for {kind} does not match its kind")]
struct PayloadDecodeError {
    kind: &'static str,
    #[source]
    source: serde_json::Error,
}

#[derive(Debug, thiserror::Error)]
#[error("adapter connection {connection_id} no longer exists")]
struct ConnectionGone {
    connection_id: String,
}
