//! File storage — S3 objects and their metadata rows.
//!
//! The `file_metadata` table is declared in this crate's own migrations, and
//! four bands above reach for the same S3 operations, so the store sits here.
//! Its HTTP surface needs the identity extractors and lives in the backend.

pub mod dao;
pub mod error;
pub mod models;
pub mod s3;
pub mod store;
