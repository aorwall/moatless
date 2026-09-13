//! Function-shaped registration helpers for typed Adapter delivery work.
//!
//! Provider modules own their payloads, kind names, and runtime functions; this
//! module owns the wrapper mechanics — decode the JSON payload once and
//! classify malformed rows as permanent.
//!
//! The handler carries no application state — the only port touched is the
//! explicit `ConnectionLookup` passed to `register_connection_delivery`.

use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

use moatless_adapter_model::connections::ConnectionLookup;
use moatless_adapter_model::error::SharedAdapterError;
use moatless_types::adapter::AdapterConnection;

use moatless_adapter_model::delivery::{
    AdapterDelivery, AdapterDeliveryFailure, AdapterDeliveryHandler, AdapterDeliveryHandlers,
};

/// A provider-owned payload whose work is scoped to one adapter connection.
pub trait ConnectionDeliveryPayload: serde::de::DeserializeOwned + Send + Sync + 'static {
    fn connection_id(&self) -> &str;
}

/// Register a typed handler that receives the decoded provider payload.
///
/// The `execute` closure carries whatever it needs (captured at registration);
/// this helper only adds the payload decode and handler wrapper.
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

/// Register a typed handler for work that re-reads its connection before
/// running provider behavior.
///
/// The connection is re-read through the `adapter_connections` port passed
/// here, in the system scope, since a durable delivery runs in the worker with
/// no user session.
pub fn register_connection_delivery<P, F, Fut>(
    handlers: &mut AdapterDeliveryHandlers,
    kind: &'static str,
    adapter_connections: Arc<dyn ConnectionLookup>,
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
    adapter_connections: &dyn ConnectionLookup,
    connection_id: &str,
) -> Result<AdapterConnection, AdapterDeliveryFailure> {
    adapter_connections
        .by_id_trusted(connection_id)
        .await
        .map_err(|e| AdapterDeliveryFailure::retryable(SharedAdapterError::Store(e)))?
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
