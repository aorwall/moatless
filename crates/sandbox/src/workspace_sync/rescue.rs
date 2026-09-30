//! Upload a stopped workspace without loading the live backend configuration.

use tracing::info;

use crate::config::RescueConfig;

/// Upload a stopped sandbox workspace to `config.put_url`.
pub async fn rescue(config: RescueConfig) -> Result<(), Box<dyn std::error::Error>> {
    let RescueConfig {
        task_id,
        workspace_dir,
        put_url,
    } = config;
    info!(task_id = %task_id, workspace_dir = %workspace_dir, "Rescuing workspace to S3");
    let bytes = super::rescue_upload(&workspace_dir, &put_url).await?;
    info!(task_id = %task_id, bytes, "Workspace rescued");
    Ok(())
}

#[cfg(test)]
mod oneshot_tests {
    use super::*;

    /// The rescue pod never sets `BACKEND_GRPC_URL` (`rescue_pod.rs` carries no
    /// backend client), so a rescue must fail on something specific to
    /// the sync itself, not on a missing backend URL it never needed.
    #[tokio::test]
    async fn oneshot_sync_does_not_require_backend_grpc_url() {
        let tmp = tempfile::TempDir::new().unwrap();
        let missing_workspace = tmp.path().join("workspace");

        std::env::remove_var("BACKEND_GRPC_URL");
        std::env::set_var("TASK_ID", "oneshot-test-task");
        std::env::set_var("WORKSPACE_DIR", missing_workspace.to_str().unwrap());
        std::env::set_var("WORKSPACE_SYNC_PUT_URL", "http://127.0.0.1:1/put");

        let error = rescue(RescueConfig::from_env().unwrap())
            .await
            .expect_err("an unarchivable workspace is still an error");

        assert!(
            !error.to_string().contains("BACKEND_GRPC_URL"),
            "a rescue must not require BACKEND_GRPC_URL: {error}"
        );

        std::env::remove_var("TASK_ID");
        std::env::remove_var("WORKSPACE_DIR");
        std::env::remove_var("WORKSPACE_SYNC_PUT_URL");
    }
}
