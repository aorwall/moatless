// Allow unwrap/expect in test code — production code still enforces the deny.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! S3 object operations and presigning.
//!
//! A leaf over `aws-sdk-s3` with no in-workspace dependency, so the SDK
//! compiles in parallel with the band graph instead of gating it.

mod error;
mod objects;
mod presign;
#[cfg(feature = "restic-backend")]
mod restic_backend;
#[cfg(feature = "sts")]
mod session;
mod storage;

#[cfg(test)]
mod objects_test;
#[cfg(all(test, feature = "restic-backend"))]
mod restic_backend_test;

pub use error::S3Error;
pub use objects::{
    delete_object, delete_objects_by_prefix, get_object, get_object_opt, get_object_range,
    head_object_modified_at, head_object_size, list_objects_with_size, prefix_has_any_object,
    put_object,
};
pub use presign::{
    PresignOp, PresignedUrl, presign_batch, presign_delete, presign_get, presign_put,
};
#[cfg(feature = "restic-backend")]
pub use restic_backend::ResticS3Backend;
#[cfg(feature = "sts")]
pub use session::{ScopedSession, assume_role_for_prefix};
pub use storage::S3Storage;
