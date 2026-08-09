//! Permanent facade re-exporting the `event_ingress` module from
//! `moatless-tasks`.
//!
//! Explicit named re-exports (never a glob) so the crate census can resolve
//! edges through the facade.

pub use moatless_tasks::event_ingress::{
    describe_ingress_metrics, is_error_sub_type, outbox, relay, AdmissionError, AdmissionRequest,
    EventAdmission,
};
