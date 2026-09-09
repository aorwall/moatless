//! Uploads whose metadata is held in memory and whose bytes are nowhere.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use moatless_types::access_control::Scope;

use super::dao::FileDao;
use super::models::{FileInfo, FileMetadata};
use super::operations::Files;
use crate::config::S3Config;

/// [`Files`] over `rows` and no object storage: what a caller that only reads
/// names, sizes and mime types needs to be tested against.
pub fn files_from(rows: Vec<FileInfo>) -> Arc<Files> {
    let store = InMemoryFiles::default();
    for row in rows {
        store.insert(row);
    }
    Arc::new(Files::new(
        Arc::new(store),
        None,
        Arc::new(S3Config {
            bucket_name: None,
            key_prefix: "uploads".to_string(),
            transcript_prefix: "transcripts".to_string(),
            workspace_prefix: "workspaces".to_string(),
            region: "us-east-1".to_string(),
        }),
    ))
}

#[derive(Default)]
struct InMemoryFiles(Mutex<HashMap<String, FileMetadata>>);

impl InMemoryFiles {
    fn insert(&self, row: FileInfo) {
        #[allow(clippy::unwrap_used, reason = "a poisoned test double has no recovery")]
        let mut rows = self.0.lock().unwrap();
        rows.insert(
            row.file_id.clone(),
            FileMetadata {
                file_id: row.file_id,
                filename: row.filename,
                mime_type: row.mime_type,
                size: row.size,
                scope: Scope::User,
                owner_user_id: None,
                created_by: None,
                uploaded_at: chrono::Utc::now(),
            },
        );
    }

    #[allow(clippy::unwrap_used, reason = "a poisoned test double has no recovery")]
    fn rows(&self) -> std::sync::MutexGuard<'_, HashMap<String, FileMetadata>> {
        self.0.lock().unwrap()
    }
}

#[async_trait]
impl FileDao for InMemoryFiles {
    async fn insert_metadata(&self, row: &FileMetadata) -> Result<(), sqlx::Error> {
        self.rows().insert(row.file_id.clone(), row.clone());
        Ok(())
    }

    async fn find_by_id(&self, file_id: &str) -> Result<Option<FileMetadata>, sqlx::Error> {
        Ok(self.rows().get(file_id).cloned())
    }

    async fn find_by_ids(
        &self,
        file_ids: &[String],
    ) -> Result<HashMap<String, FileMetadata>, sqlx::Error> {
        let rows = self.rows();
        Ok(file_ids
            .iter()
            .filter_map(|id| rows.get(id).map(|row| (id.clone(), row.clone())))
            .collect())
    }

    async fn delete_metadata(&self, file_id: &str) -> Result<bool, sqlx::Error> {
        Ok(self.rows().remove(file_id).is_some())
    }
}
