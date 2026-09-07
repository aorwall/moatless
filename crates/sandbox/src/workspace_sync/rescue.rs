//! Upload a stopped workspace without loading the live backend configuration.

use tracing::info;

/// Upload a stopped sandbox workspace using the rescue environment.
pub async fn rescue_from_env() -> Result<(), Box<dyn std::error::Error>> {
    let task_id = std::env::var("TASK_ID").map_err(|_| "SANDBOX_ONESHOT=sync requires TASK_ID")?;
    let workspace_dir = std::env::var("WORKSPACE_DIR").unwrap_or_else(|_| "/workspace".to_string());
    let put_url = std::env::var("WORKSPACE_SYNC_PUT_URL")
        .map_err(|_| "SANDBOX_ONESHOT=sync requires WORKSPACE_SYNC_PUT_URL")?;
    info!(task_id = %task_id, workspace_dir = %workspace_dir, "Rescuing workspace to S3");
    let bytes = super::rescue_upload(&workspace_dir, &put_url).await?;
    info!(task_id = %task_id, bytes, "Workspace rescued");
    Ok(())
}

#[cfg(test)]
mod oneshot_tests {
    use super::*;

    /// The rescue pod never sets `BACKEND_GRPC_URL` (`rescue_pod.rs` carries no
    /// backend client), so `rescue_from_env` must fail on something specific to the
    /// sync itself, not on a missing backend URL it never needed.
    #[tokio::test]
    async fn oneshot_sync_does_not_require_backend_grpc_url() {
        let tmp = tempfile::TempDir::new().unwrap();
        let missing_workspace = tmp.path().join("workspace");

        std::env::remove_var("BACKEND_GRPC_URL");
        std::env::set_var("TASK_ID", "oneshot-test-task");
        std::env::set_var("WORKSPACE_DIR", missing_workspace.to_str().unwrap());
        std::env::set_var("WORKSPACE_SYNC_PUT_URL", "http://127.0.0.1:1/put");

        let error = rescue_from_env()
            .await
            .expect_err("an unarchivable workspace is still an error");

        assert!(
            !error.to_string().contains("BACKEND_GRPC_URL"),
            "rescue_from_env must not require BACKEND_GRPC_URL: {error}"
        );

        std::env::remove_var("TASK_ID");
        std::env::remove_var("WORKSPACE_DIR");
        std::env::remove_var("WORKSPACE_SYNC_PUT_URL");
    }
}
