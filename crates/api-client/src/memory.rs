//! The memory routes: entries a Task keeps for later Tasks, by scope.

use moatless_api_schemas::memory::{
    AppendMemoryRequest, ClaimMemoryRequest, GetMemoryResponse, ListMemoryResponse,
    MemoryEntryResponse, MemoryKeyRequest, MemoryLogResponse, MemoryScope, PutMemoryRequest,
};

use crate::ApiError;
use crate::client::{ApiClient, urlenc};

/// Narrowing for [`ApiClient::list_memory`].
#[derive(Debug, Default)]
pub struct MemoryListQuery<'a> {
    pub scope: Option<MemoryScope>,
    pub prefix: Option<&'a str>,
    pub tag: Option<&'a str>,
    pub updated_since_seconds: Option<i64>,
    pub limit: Option<i64>,
}

fn key_pairs(key: &str, scope: Option<MemoryScope>) -> Vec<(&'static str, Option<String>)> {
    vec![
        ("key", Some(key.to_string())),
        ("scope", scope.map(|s| s.as_str().to_string())),
    ]
}

/// `/api/v1/tasks/{task_id}/memory{suffix}` with the set `pairs` as a query.
fn memory_path(task_id: &str, suffix: &str, pairs: &[(&str, Option<String>)]) -> String {
    let query: Vec<String> = pairs
        .iter()
        .filter_map(|(name, value)| value.as_ref().map(|v| format!("{name}={}", urlenc(v))))
        .collect();
    let mut path = format!("/api/v1/tasks/{}/memory{suffix}", urlenc(task_id));
    if !query.is_empty() {
        path.push('?');
        path.push_str(&query.join("&"));
    }
    path
}

impl ApiClient {
    /// GET `/api/v1/tasks/{task_id}/memory` — summaries, newest first.
    pub async fn list_memory(
        &self,
        task_id: &str,
        query: &MemoryListQuery<'_>,
    ) -> Result<ListMemoryResponse, ApiError> {
        let pairs = [
            ("scope", query.scope.map(|s| s.as_str().to_string())),
            ("prefix", query.prefix.map(str::to_string)),
            ("tag", query.tag.map(str::to_string)),
            (
                "updatedSinceSeconds",
                query.updated_since_seconds.map(|n| n.to_string()),
            ),
            ("limit", query.limit.map(|n| n.to_string())),
        ];
        self.get(&memory_path(task_id, "", &pairs)).await
    }

    /// GET `/api/v1/tasks/{task_id}/memory/entry` — an entry, or every match
    /// of a `prefix*`.
    pub async fn get_memory(
        &self,
        task_id: &str,
        key: &str,
        scope: Option<MemoryScope>,
    ) -> Result<GetMemoryResponse, ApiError> {
        self.get(&memory_path(task_id, "/entry", &key_pairs(key, scope)))
            .await
    }

    /// PUT `/api/v1/tasks/{task_id}/memory/entry`
    pub async fn put_memory(
        &self,
        task_id: &str,
        req: &PutMemoryRequest,
    ) -> Result<MemoryEntryResponse, ApiError> {
        self.put(&memory_path(task_id, "/entry", &[]), req).await
    }

    /// DELETE `/api/v1/tasks/{task_id}/memory/entry`
    pub async fn delete_memory(
        &self,
        task_id: &str,
        key: &str,
        scope: Option<MemoryScope>,
    ) -> Result<(), ApiError> {
        self.delete_request(&memory_path(task_id, "/entry", &key_pairs(key, scope)))
            .await
    }

    /// GET `/api/v1/tasks/{task_id}/memory/search` — summaries, best first.
    pub async fn search_memory(
        &self,
        task_id: &str,
        query: &str,
        scope: Option<MemoryScope>,
        limit: Option<i64>,
    ) -> Result<ListMemoryResponse, ApiError> {
        let pairs = [
            ("q", Some(query.to_string())),
            ("scope", scope.map(|s| s.as_str().to_string())),
            ("limit", limit.map(|n| n.to_string())),
        ];
        self.get(&memory_path(task_id, "/search", &pairs)).await
    }

    /// GET `/api/v1/tasks/{task_id}/memory/log` — history, newest first.
    pub async fn memory_log(
        &self,
        task_id: &str,
        key: &str,
        scope: Option<MemoryScope>,
        limit: Option<i64>,
    ) -> Result<MemoryLogResponse, ApiError> {
        let mut pairs = key_pairs(key, scope);
        pairs.push(("limit", limit.map(|n| n.to_string())));
        self.get(&memory_path(task_id, "/log", &pairs)).await
    }

    /// POST `/api/v1/tasks/{task_id}/memory/append`
    pub async fn append_memory(
        &self,
        task_id: &str,
        req: &AppendMemoryRequest,
    ) -> Result<MemoryEntryResponse, ApiError> {
        self.post(&memory_path(task_id, "/append", &[]), req).await
    }

    /// POST `/api/v1/tasks/{task_id}/memory/verify`
    pub async fn verify_memory(
        &self,
        task_id: &str,
        req: &MemoryKeyRequest,
    ) -> Result<MemoryEntryResponse, ApiError> {
        self.post(&memory_path(task_id, "/verify", &[]), req).await
    }

    /// POST `/api/v1/tasks/{task_id}/memory/claim`
    pub async fn claim_memory(
        &self,
        task_id: &str,
        req: &ClaimMemoryRequest,
    ) -> Result<MemoryEntryResponse, ApiError> {
        self.post(&memory_path(task_id, "/claim", &[]), req).await
    }

    /// POST `/api/v1/tasks/{task_id}/memory/release`
    pub async fn release_memory(
        &self,
        task_id: &str,
        req: &MemoryKeyRequest,
    ) -> Result<(), ApiError> {
        let url = format!(
            "{}{}",
            self.base_url(),
            memory_path(task_id, "/release", &[])
        );
        let resp = self.http().post(&url).json(req).send().await?;
        self.check_status(resp).await
    }
}
