//! Shared ingress helpers for provider dispatchers.
//!
//! Providers keep transport verification, parsing, and provider-specific
//! decisions. This module owns the common "durably admit this work and report
//! whether it was new or duplicate" step.

use tracing::{debug, warn};

use crate::adapters::delivery::{AdapterDeliveryRequest, Admission, AdmitError};
use crate::state::AppState;

/// Commit one provider work item to the durable adapter delivery queue.
pub(crate) async fn admit_delivery(
    state: &AppState,
    request: AdapterDeliveryRequest,
) -> Result<Admission, AdmitError> {
    state.adapter_delivery.admit(request).await
}

/// Emit a consistent operator trace for provider delivery admission.
pub(crate) fn log_admission(
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
