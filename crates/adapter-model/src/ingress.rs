//! Shared ingress vocabulary for provider dispatchers.
//!
//! Providers keep transport verification, parsing, and provider-specific
//! decisions — every Adapter writes its own `receive`, since every step inside
//! one is Adapter policy. What they share is the middle step and how a delivery
//! *ends*: [`admit_delivery`] hands work over through the [`DeliveryAdmission`]
//! port, [`Outcome`]/[`IngressError`] name the transport treatments, and
//! [`log_admission`] gives one operator trace.

use tracing::{debug, warn};

use crate::delivery::{AdapterDeliveryRequest, Admission, AdmitError, DeliveryAdmission};
use crate::error::AdapterError;

/// Commit one provider work item to the durable adapter delivery queue.
///
/// Returns [`AdmitError`] unflattened rather than [`AdapterError`]: a `receive`
/// that `?`s it lands on [`IngressError::NotAdmitted`], the one class that must
/// never be acknowledged.
pub async fn admit_delivery(
    admission: &dyn DeliveryAdmission,
    request: AdapterDeliveryRequest,
) -> Result<Admission, AdmitError> {
    admission.admit(request).await
}

/// What one Adapter's `receive` made of a delivery it is prepared to
/// acknowledge.
///
/// Both variants answer `2xx`: durably-owned work is metered by the delivery
/// worker when it runs, while an ignored delivery is metered here, carrying its
/// own labels since ingress is the last place that knows them.
/// [`Self::Ignored`]'s fields an Adapter never labels are left empty — see
/// [`Self::ignored`].
#[derive(Debug)]
pub enum Outcome {
    /// The delivery's work is durably owned: admitted by this delivery, or
    /// already admitted by an earlier copy of it.
    Processed {
        /// The Sender's own event name, empty for Senders that never labelled
        /// their webhook metric.
        event_type: String,
    },
    /// The delivery carried nothing to route, and there is no work left to lose
    /// by acknowledging it.
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
/// The three variants get three different transport treatments, and getting the
/// class wrong is how a delivery is lost. Each Adapter keeps its own match: the
/// *type* is shared, the handler policy is not — e.g. Slack answers
/// [`Self::Ingress`] with the failure's own status while GitHub PR meters it
/// and returns `200`.
#[derive(Debug, thiserror::Error)]
pub enum IngressError {
    /// Durable admission did not commit, so the backend owns nothing and a
    /// `200` would be a lie.
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
    /// Deliberately **not** `#[from]`: Telegram's poller holds its offset back
    /// for this class, so a `?` reaching it by inference (rather than a
    /// deliberate decision) wedges the poller on one update and blocks
    /// everything queued behind it. Every construction is written out; a
    /// provider-raised error is classified at its raise site instead.
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
