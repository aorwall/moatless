//! The record collection routes: records a Task keeps in a scope it reaches.

use moatless_api_schemas::artifact::WriteArtifactRecordRequest;
use moatless_api_schemas::collection::{
    CollectionRecordResponse, ListCollectionRecordsResponse, ListCollectionsResponse, MemoryScope,
};

use crate::ApiError;
use crate::artifacts::query;
use crate::client::{ApiClient, urlenc};

impl ApiClient {
    /// GET `/api/v1/tasks/{task_id}/collections`; `scope` defaults to the
    /// Task's Loop, else the Task.
    pub async fn list_collections(
        &self,
        task_id: &str,
        scope: Option<MemoryScope>,
    ) -> Result<ListCollectionsResponse, ApiError> {
        let query = query(&[("scope", scope.map(MemoryScope::as_str))]);
        self.get(&format!(
            "/api/v1/tasks/{}/collections{query}",
            urlenc(task_id)
        ))
        .await
    }

    /// GET `/api/v1/tasks/{task_id}/collections/{name}/records`, filtered by
    /// key `prefix` and change time `since` when given.
    pub async fn list_collection_records(
        &self,
        task_id: &str,
        name: &str,
        scope: Option<MemoryScope>,
        prefix: Option<&str>,
        since: Option<&str>,
    ) -> Result<ListCollectionRecordsResponse, ApiError> {
        let query = query(&[
            ("scope", scope.map(MemoryScope::as_str)),
            ("prefix", prefix),
            ("since", since),
        ]);
        self.get(&format!(
            "/api/v1/tasks/{}/collections/{}/records{query}",
            urlenc(task_id),
            urlenc(name)
        ))
        .await
    }

    /// POST `/api/v1/tasks/{task_id}/collections/{name}/records`
    pub async fn write_collection_record(
        &self,
        task_id: &str,
        name: &str,
        scope: Option<MemoryScope>,
        req: &WriteArtifactRecordRequest,
    ) -> Result<CollectionRecordResponse, ApiError> {
        let query = query(&[("scope", scope.map(MemoryScope::as_str))]);
        self.post(
            &format!(
                "/api/v1/tasks/{}/collections/{}/records{query}",
                urlenc(task_id),
                urlenc(name)
            ),
            req,
        )
        .await
    }
}
