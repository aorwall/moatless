//! Generated gRPC types and services for sandbox <-> backend communication.

pub mod client;

pub mod sandbox {
    pub mod v1 {
        // Generated protobuf message enums carry size-disparate variants by
        // design (a bare bool beside a full result message); boxing them to
        // placate the lint would only churn every construction site.
        #![allow(
            clippy::all,
            clippy::pedantic,
            clippy::too_long_first_doc_paragraph,
            reason = "prost and tonic generate this module"
        )]
        tonic::include_proto!("sandbox.v1");
    }
}

// Re-export for convenience
pub use sandbox::v1::*;

mod event_conversion;
mod workspace_path;

pub use event_conversion::UnrepresentableEventType;
pub use moatless_types::event;
pub use workspace_path::SNAPSHOT_ROOT;
