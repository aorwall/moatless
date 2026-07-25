//! Regression: creating a task after `git_sync` has elected a primary placement
//! must not violate `workspace_repos_one_primary`.
//!
//! A repo's shadow workspace (`ws_<repo>`) is written by two paths:
//!   - `ensure_shadow_workspace` (task create) — deterministic id `wsr_<repo>`.
//!   - `git_sync` — deletes every placement and re-inserts the primary under a
//!     random id (`wsr_<uuid>`).
//! Once git_sync has run, the deterministic id is gone, so a plain
//! `ON CONFLICT (id)` on the ensure insert no longer dedupes and would add a
//! second primary → 500. The ensure insert now skips when a primary already
//! exists, so the second task create succeeds.

use integration_tests::{create_repo_request, exec_psql, query_psql, test_client};
use moatless_api_schemas::{enums::RepositoryKind, task::CreateTaskRequest};

fn task_req(repo_id: &str, name: &str) -> CreateTaskRequest {
    CreateTaskRequest {
        repository_id: Some(repo_id.to_string()),
        workspace_id: None,
        message: Some("shadow-primary regression".into()),
        name: Some(name.into()),
        branch: None,
        description: None,
        agent_mode: None,
        agent_type: None,
        contexts: vec![],
        file_ids: vec![],
        model: None,
        headless: None,
        parent_task_id: None,
    }
}

#[tokio::test]
async fn task_create_survives_git_sync_reelected_primary() {
    let client = test_client().await;

    // A fresh repo → its own clean shadow workspace `ws_<repo_id>`.
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut repo_req = create_repo_request(
        &format!("shadow-primary-regression-{unique}"),
        RepositoryKind::Template,
    );
    repo_req.remote_url = Some(format!(
        "https://github.com/aorwall/shadow-primary-regression-{unique}.git"
    ));
    let repo_id = client.api().create_repository(&repo_req).await.unwrap().id;
    let ws_id = format!("ws_{repo_id}");

    // First task creates the shadow workspace with the deterministic primary.
    client
        .api()
        .create_task(&task_req(&repo_id, "first"))
        .await
        .unwrap();

    // Simulate git_sync: drop every placement, re-elect the primary under a
    // random id — exactly the state that used to break the next create.
    exec_psql(&format!(
        "DELETE FROM workspace_repos WHERE workspace_id = '{ws_id}'; \
         INSERT INTO workspace_repos (id, workspace_id, repository_id, is_primary, position) \
         VALUES ('wsr_' || md5(random()::text), '{ws_id}', '{repo_id}', true, 0);"
    ));

    // Regression: this used to 500 with `workspace_repos_one_primary`.
    client
        .api()
        .create_task(&task_req(&repo_id, "second"))
        .await
        .unwrap();

    // The workspace still has exactly one primary.
    let primaries = query_psql(&format!(
        "SELECT count(*) FROM workspace_repos WHERE workspace_id = '{ws_id}' AND is_primary"
    ));
    assert_eq!(primaries, "1");
}
