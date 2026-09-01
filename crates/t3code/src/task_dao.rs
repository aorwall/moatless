use async_trait::async_trait;
use moatless_types::access_control::ScopeContext;
use moatless_types::task::TaskStatus;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskView {
    #[sqlx(try_from = "String")]
    pub status: TaskStatus,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait T3TaskDao: Send + Sync {
    async fn find_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskView>, sqlx::Error>;

    /// The same read, narrowed to Tasks this viewer may **operate**: the write
    /// rule widened by `visibility = 'public'`. `None` when the Task is gone,
    /// unreadable, or readable but not the viewer's to run — one answer,
    /// because telling the last case apart on the wire would say who owns it.
    async fn controlled_by_id(
        &self,
        task_id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskView>, sqlx::Error>;
}
