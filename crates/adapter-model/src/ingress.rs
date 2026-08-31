//! Shared ingress vocabulary for provider dispatchers.
//!
//! Providers keep transport verification, parsing, and provider-specific
//! decisions — every Adapter writes its own `receive`, because every step
//! inside one is Adapter policy. What they share is the one step in the middle
//! and how a delivery *ends*: [`admit_delivery`] hands work over through the
//! [`DeliveryAdmission`] port, and [`Outcome`] and [`IngressError`] name the
//! transport treatments while [`log_admission`] gives admission one operator
//! trace instead of five.
//!
//! Four Adapters invented these two enums independently — GitHub PR and Linear
//! first, then Slack and Telegram in #174, deliberately written to be identical
//! so this hoist would be a deletion. See Decision 2 of
//! `.plans/remaining-adapter-breakup.md`.

use tracing::{debug, warn};

use crate::delivery::{AdapterDeliveryRequest, Admission, AdmitError, DeliveryAdmission};
use crate::error::AdapterError;

/// Commit one provider work item to the durable adapter delivery queue.
///
/// The failure is handed back as [`AdmitError`], not flattened into an
/// [`AdapterError`]: a `receive` that `?`s it lands on
/// [`IngressError::NotAdmitted`], the one class that must never be
/// acknowledged. Keeping it unflattened is what preserves that distinction to
/// the caller.
pub async fn admit_delivery(
    admission: &dyn DeliveryAdmission,
    request: AdapterDeliveryRequest,
) -> Result<Admission, AdmitError> {
    admission.admit(request).await
}

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
    /// the seam's admission call, which hands back [`AdmitError`] unflattened.
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
