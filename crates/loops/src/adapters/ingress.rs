//! Durable admission for provider dispatchers.
//!
//! The shared ingress vocabulary — `Outcome`, `IngressError`, `log_admission` —
//! is the Adapter kernel's (`moatless_adapter_model::ingress`). What is here is
//! the one step that needs this band's store: committing a delivery to the
//! durable queue.

use crate::adapters::delivery::AdapterDeliveryStore;
use moatless_adapter_model::delivery::{AdapterDeliveryRequest, Admission, AdmitError};

/// Commit one provider work item to the durable adapter delivery queue.
///
/// The failure is handed back as [`AdmitError`], not flattened into an
/// `AdapterError`: a `receive` that `?`s it lands on
/// `IngressError::NotAdmitted`, which is the one class that must never be
/// acknowledged. Flattening it first is what made that variant unreachable for
/// three of the four Adapters before the vocabulary was shared.
pub async fn admit_delivery(
    store: &AdapterDeliveryStore,
    request: AdapterDeliveryRequest,
) -> Result<Admission, AdmitError> {
    store.admit(request).await
}

#[cfg(test)]
mod tests {
    use super::admit_delivery;
    use crate::adapters::delivery::AdapterDeliveryStore;
    use moatless_adapter_model::delivery::{AdapterDeliveryRequest, AdmitError};
    use moatless_adapter_model::ingress::IngressError;
    use moatless_core::db::DbPool;

    /// A store whose database is not there, so `admit` fails for a reason no
    /// mock has to be talked into.
    fn store_with_no_database() -> AdapterDeliveryStore {
        let pool: DbPool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_millis(10))
            .connect_lazy("postgres://fake:fake@127.0.0.1:1/fake")
            .unwrap();
        AdapterDeliveryStore::new(pool)
    }

    /// A refused admission reaches a `receive` as [`IngressError::NotAdmitted`].
    ///
    /// This is the one step every `receive` shares, and the `?` below is the one
    /// each of them writes — so what it pins is that the class survives the
    /// admission step rather than arriving pre-flattened. Telegram and the
    /// generic Webhook Adapter admitted through `admit_inbound_message` until
    /// this hoist, which did `.admit(..).await?` internally and handed back an
    /// `AdapterError`; `NotAdmitted` was unreachable for both, and any future
    /// helper that flattens again fails here rather than silently downgrading a
    /// delivery nothing was written for into one the poller retries forever.
    #[tokio::test]
    async fn a_refused_admission_reaches_receive_as_not_admitted() {
        let request = AdapterDeliveryRequest {
            adapter: "telegram",
            kind: "telegram.inbound_message",
            event_id: "1".into(),
            payload: serde_json::json!({}),
        };

        let received: Result<_, IngressError> =
            async { Ok(admit_delivery(&store_with_no_database(), request).await?) }.await;

        assert!(
            matches!(
                received,
                Err(IngressError::NotAdmitted(AdmitError::Store(_)))
            ),
            "a refused admission must not arrive as an ingress rejection"
        );
    }
}
