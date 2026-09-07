//! File storage metadata — the `file_metadata` row and its DAO.
//!
//! The table is declared in this crate's own migrations and the task band reads
//! it, so the metadata sits here. The S3 operations are `moatless-s3`: a leaf
//! over `aws-sdk-s3` that nothing below band 8 names, which keeps that SDK off
//! this crate's compile path. Storing an upload needs both halves plus the
//! multipart route, so it lives with the handlers in the backend.

pub mod dao;
pub mod models;
