// Allow unwrap/expect in test code — production code still enforces the deny.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! S3 object operations and presigning.
//!
//! A leaf over `aws-sdk-s3` with no in-workspace dependency, so the SDK
//! compiles in parallel with the band graph instead of gating it.

mod error;
mod objects;
mod presign;
mod storage;

#[cfg(test)]
mod objects_test;

pub use error::S3Error;
pub use objects::{
    delete_object, delete_objects_by_prefix, get_object, get_object_opt, head_object_modified_at,
    put_object,
};
pub use presign::{PresignedUrl, presign_delete, presign_get, presign_put};
pub use storage::S3Storage;
