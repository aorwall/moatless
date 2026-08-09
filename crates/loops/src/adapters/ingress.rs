//! Shared ingress vocabulary and helpers for provider dispatchers.
//!
//! Providers keep transport verification, parsing, and provider-specific
//! decisions — every Adapter writes its own `receive`, because every step
//! inside one is Adapter policy. What they share is how a delivery *ends*:
//! [`Outcome`] and [`IngressError`] name the transport treatments, and
//! [`admit_delivery`] is the one "durably admit this work and report whether it
//! was new or duplicate" step.
//!
//! Four Adapters invented these two enums independently — GitHub PR and Linear
//! first, then Slack and Telegram in #174, deliberately written to be identical
//! so this hoist would be a deletion. See Decision 2 of
//! `.plans/remaining-adapter-breakup.md`.

use tracing::{debug, warn};

use crate::adapters::delivery::{
    AdapterDeliveryRequest, AdapterDeliveryStore, Admission, AdmitError,
};
use crate::adapters::error::AdapterError;

/// What one Adapter's `receive` made of a delivery it is prepared to
/// acknowledge.
///
/// Both variants are answered `2xx`. What they differ in is what the handler
/// records: a delivery whose work is now durably owned is metered by the
/// delivery worker when that work runs, and an ignored one is metered here,
/// under labels the outcome has to carry because ingress is the last place that
/// knows them.
///
/// The four fields on [`Self::Ignored`] are `record_webhook`'s label set minus
/// the outcome itself. Adapters that never labelled a dimension leave it empty,
/// which is what they passed before this type existed — see [`Self::ignored`].
#[derive(Debug)]
pub enum Outcome {
    /// The delivery's work is durably owned: admitted by this delivery, or
    /// already admitted by an earlier copy of it.
    Processed {
        /// The Sender's own event name, empty for Senders that never labelled
        /// their webhook metric.
        event_type: String,
    },
    /// The delivery carried nothing to route, and there is no work left to
    /// lose by acknowledging it.
    Ignored {
        event_type: String,
        /// A fixed-vocabulary metric label, not a message.
        reason: &'static str,
        /// The routing pair the ignore is attributed to, empty when the
        /// delivery was dropped before one was resolved.
        subscription_id: String,
        connection_id: String,
    },
}

impl Outcome {
    /// An ignore attributed to no routing pair — either because none was
    /// resolved, or because this Adapter has never labelled one.
    pub fn ignored(event_type: impl Into<String>, reason: &'static str) -> Self {
        Self::Ignored {
            event_type: event_type.into(),
            reason,
            subscription_id: String::new(),
            connection_id: String::new(),
        }
    }
}

/// Why a delivery did not reach an [`Outcome`].
///
/// The three variants exist because they get three different transport
/// treatments, and getting the class wrong is how a delivery is lost. Each
/// Adapter keeps its own match on them: the *type* is shared, the handler
/// policy is not — Slack answers [`Self::Ingress`] with the failure's own
/// status while GitHub PR meters it and returns `200`, and both did so before
/// this type existed.
#[derive(Debug, thiserror::Error)]
pub enum IngressError {
    /// Durable admission did not commit, so the backend owns nothing and a
    /// `200` would be a lie. Every Adapter reaches this by admitting through
    /// [`admit_delivery`], which hands back [`AdmitError`] unflattened.
    ///
    /// `#[from]` is safe here and only here: [`AdmitError`] has one class, so
    /// there is no decision for a `?` to make on the caller's behalf.
    #[error("adapter delivery was not admitted")]
    NotAdmitted(#[from] AdmitError),
    /// The delivery carries no field that could tell its work apart from the
    /// next delivery's, or none that names what it is. No re-delivery supplies
    /// one, so it is a `400` rather than something to try again.
    #[error("{0}")]
    Unidentifiable(String),
    /// Everything the ingress path itself rejected or failed on: a missing
    /// header, an unparseable body, a refused signature, a database read.
    ///
    /// Deliberately **not** `#[from]`. This is the class Telegram's poller
    /// holds its offset back for, so a `?` that reached it by inference —
    /// rather than because someone decided this failure is worth another
    /// delivery — wedges the poller on one update and blocks every update
    /// queued behind it. That shipped once already, on a permanently malformed
    /// envelope normalization raises as `BadRequest`. Every construction is
    /// written out, and every new step in a `receive` has to write one too;
    /// where a provider-raised error is classified, that belongs at the raise
    /// site (`telegram::ingress::classify_normalization` is the shape), not in
    /// the poller's rule, which stays a match on these three variants and
    /// nothing else.
    #[error(transparent)]
    Ingress(AdapterError),
}

/// Commit one provider work item to the durable adapter delivery queue.
///
/// The failure is handed back as [`AdmitError`], not flattened into an
/// [`AdapterError`]: a `receive` that `?`s it lands on
/// [`IngressError::NotAdmitted`], which is the one class that must never be
/// acknowledged. Flattening it first is what made that variant unreachable for
/// three of the four Adapters before the vocabulary was shared.
pub async fn admit_delivery(
    store: &AdapterDeliveryStore,
    request: AdapterDeliveryRequest,
) -> Result<Admission, AdmitError> {
    store.admit(request).await
}

/// Emit a consistent operator trace for provider delivery admission.
pub fn log_admission(
    adapter: &'static str,
    kind: &'static str,
    event_id: &str,
    admission: Admission,
) {
    match admission {
        Admission::Admitted => {
            debug!(adapter, kind, event_id, "Adapter delivery admitted");
        }
        Admission::Duplicate => {
            warn!(
                adapter,
                kind, event_id, "Adapter delivery already admitted; nothing re-run"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{admit_delivery, IngressError};
    use crate::adapters::delivery::{AdapterDeliveryRequest, AdapterDeliveryStore, AdmitError};
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
