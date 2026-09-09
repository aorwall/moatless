//! Owner construction for the upload operations.

use std::sync::Arc;

use moatless_s3::S3Storage;

use super::dao::PgFileDao;
use super::operations::Files;
use crate::config::S3Config;
use crate::db::DbPool;

/// Build the uploads over the shared pool and the deployment's object storage.
pub fn files(pool: DbPool, s3: Option<S3Storage>, s3_config: Arc<S3Config>) -> Arc<Files> {
    Arc::new(Files::new(Arc::new(PgFileDao::new(pool)), s3, s3_config))
}
