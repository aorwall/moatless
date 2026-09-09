//! The real-store side of durable admission.
//!
//! The admission seam is the Adapter kernel's: `DeliveryAdmission` names the
//! one call a provider makes, `moatless_adapter_model::ingress::admit_delivery`
//! wraps it, and the kernel's `FakeAdmission` drives the four outcome branches
//! at the handler level without a database. What stays here is the one
//! characterization that a fake cannot stand in for — that the *concrete*
//! `AdapterDeliveryStore`, refused by a database that is not there, surfaces as
//! `IngressError::NotAdmitted` through the adapter-model admission port.

#[cfg(test)]
mod tests {
    use crate::adapters::delivery::dao::AdapterDeliveryStore;
    use moatless_adapter_model::delivery::{AdapterDeliveryRequest, AdmitError, DeliveryAdmission};
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
            async { Ok(store_with_no_database().admit(request).await?) }.await;

        assert!(
            matches!(
                received,
                Err(IngressError::NotAdmitted(AdmitError::Store(_)))
            ),
            "a refused admission must not arrive as an ingress rejection"
        );
    }
}
