//! The `tags` and `task_tags` tables, as a host and the tag routes reach them.

use std::sync::Arc;

use super::dao::TagDao;
use super::models::{Tag, TaskTag};

/// The `tags` and `task_tags` tables, as the backend and this crate's own
/// handlers reach them.
///
/// The published seam is narrower: [`crate::ports::TagCatalog`] resolves names
/// to ids.
pub struct Tags {
    store: Arc<dyn TagDao>,
}

impl Tags {
    pub(crate) fn new(store: Arc<dyn TagDao>) -> Self {
        Self { store }
    }

    /// Tag `task_id` with `names`, creating any tag that does not exist yet.
    ///
    /// Best effort: failures are logged and swallowed, since the caller is an
    /// event handler whose message is already committed and whose tags are a
    /// listing convenience.
    pub async fn propagate_tags(&self, task_id: &str, names: &[String]) {
        match self.store.find_or_create_tags(names).await {
            Ok(tags) => {
                let tag_ids: Vec<String> = tags.into_iter().map(|t| t.tag_id).collect();
                if let Err(e) = self.store.add_task_tags(task_id, &tag_ids).await {
                    tracing::warn!(
                        task_id = %task_id,
                        error = %e,
                        "Failed to add skill tags"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    task_id = %task_id,
                    error = %e,
                    "Failed to find/create skill tags"
                );
            }
        }
    }

    /// See [`TagDao::list_all`].
    pub(crate) async fn list_all(&self) -> Result<Vec<Tag>, sqlx::Error> {
        self.store.list_all().await
    }

    /// See [`TagDao::find_or_create_tags`].
    pub(crate) async fn find_or_create_tags(
        &self,
        names: &[String],
    ) -> Result<Vec<Tag>, sqlx::Error> {
        self.store.find_or_create_tags(names).await
    }

    /// See [`TagDao::set_task_tags`].
    pub(crate) async fn set_task_tags(
        &self,
        task_id: &str,
        tag_ids: &[String],
    ) -> Result<(), sqlx::Error> {
        self.store.set_task_tags(task_id, tag_ids).await
    }

    /// See [`TagDao::get_tags_for_tasks`].
    pub(crate) async fn get_tags_for_tasks(
        &self,
        task_ids: &[String],
    ) -> Result<Vec<TaskTag>, sqlx::Error> {
        self.store.get_tags_for_tasks(task_ids).await
    }
}
