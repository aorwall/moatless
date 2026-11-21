//! The real-store side of durable admission.
//!
//! The admission seam is the Adapter kernel's: `DeliveryAdmission` names the
//! one call a provider makes, `moatless_adapter_model::ingress::admit_delivery`
//! wraps it, and the kernel's `FakeAdmission` drives the four outcome branches
//! without a database. What stays here is the one characterization a fake
//! cannot stand in for: the *concrete* `AdapterDeliveryStore`, refused by a
//! missing database, surfaces as `IngressError::NotAdmitted`.

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

    /// Every `receive` shares this `?` step, so the class must survive rather
    /// than arrive pre-flattened; a helper that flattens it again fails here
    /// instead of silently downgrading an unwritten delivery into one the
    /// sender retries forever.
    #[tokio::test]
    async fn a_refused_admission_reaches_receive_as_not_admitted() {
        let request = AdapterDeliveryRequest {
            adapter: "slack",
            kind: "slack.inbound_message",
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
