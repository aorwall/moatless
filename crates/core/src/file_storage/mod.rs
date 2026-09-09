//! Uploads: the object in S3, the `file_metadata` row that names it, and the
//! access check that decides who may read one back.
//!
//! The table is declared in this crate's own migrations, and [`Files`] is the
//! only way to it. Storing an upload writes both halves and undoes the object
//! when the row fails, so no caller assembles a put and an insert itself.
//!
//! This module is why `moatless-core` names `moatless-s3`, and so why every
//! member above core compiles `aws-sdk-s3`. `docs/deferred-changes.md` records
//! what that costs the CI cache.

pub mod compose;
mod dao;
mod error;
mod models;
mod operations;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use error::FileStorageError;
pub use models::{Attachment, Download, FileInfo, NewUpload};
pub use operations::Files;
