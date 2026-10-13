//! The production [`crate::ports::UsageReporter`].

#[async_trait::async_trait]
impl crate::ports::UsageReporter for moatless_tasks::task_turn::usage::TurnUsage {
    async fn buckets(
        &self,
        viewer: &moatless_types::access_control::ScopeContext,
        window: &crate::ports::UsageWindow,
    ) -> Result<Vec<crate::ports::UsageBucket>, moatless_types::store::StoreError> {
        moatless_tasks::task_turn::usage::TurnUsage::buckets(self, viewer, window)
            .await
            .map_err(|cause| {
                moatless_types::store::StoreError::with_source("read t3 usage buckets", cause)
            })
    }
}
