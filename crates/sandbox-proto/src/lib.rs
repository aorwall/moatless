//! Generated gRPC types and services for sandbox ↔ backend communication.

pub mod sandbox {
    pub mod v1 {
        tonic::include_proto!("sandbox.v1");
    }
}

// Re-export for convenience
pub use sandbox::v1::*;
