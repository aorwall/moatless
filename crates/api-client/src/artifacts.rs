//! The artifact routes: publishing pages, listing them, and their records.

use crate::ApiError;
use crate::client::{ApiClient, urlenc};

/// How [`ApiClient::publish_artifact`] shows the page and what it may do.
#[derive(Debug, Clone, Default)]
pub struct PageOptions {
    /// Show the page in the Task's thread.
    pub inline: Option<InlineArtifact>,
    pub data: ArtifactData,
    /// Limit the page to readers of the Task rather than any signed-in user.
    pub private: bool,
}

/// The inline fields of [`PageOptions`].
#[derive(Debug, Clone, Default)]
pub struct InlineArtifact {
    pub height: Option<i32>,
}

/// What a page may do with Artifact records.
#[derive(Debug, Clone, Copy, Default)]
pub struct ArtifactData {
    /// The page may save records as the reader viewing it.
    pub accepts: bool,
    /// Each record a reader saves queues a message to the Task's agent.
    pub notify: bool,
}

impl ApiClient {
    /// POST `/api/v1/tasks/{task_id}/artifacts` — publish one HTML page,
    /// named `file_name`, as the next version of an Artifact.
    pub async fn publish_artifact(
        &self,
        task_id: &str,
        title: &str,
        slug: Option<&str>,
        options: &PageOptions,
        file_name: &str,
        html: Vec<u8>,
    ) -> Result<moatless_api_schemas::artifact::ArtifactResponse, ApiError> {
        let url = format!(
            "{}/api/v1/tasks/{}/artifacts",
            self.base_url(),
            urlenc(task_id)
        );
        let mut form = reqwest::multipart::Form::new().text("title", title.to_string());
        if let Some(slug) = slug {
            form = form.text("slug", slug.to_string());
        }
        if let Some(inline) = &options.inline {
            form = form.text("inline", "true");
            if let Some(height) = inline.height {
                form = form.text("height", height.to_string());
            }
        }
        if options.data.accepts {
            form = form.text("data", "true");
        }
        if options.data.notify {
            form = form.text("notify", "true");
        }
        if options.private {
            form = form.text("visibility", "private");
        }
        form = form.part(
            "file",
            reqwest::multipart::Part::bytes(html).file_name(file_name.to_string()),
        );
        let resp = self.http().post(&url).multipart(form).send().await?;
        self.parse_response(resp).await
    }

    /// GET `/api/v1/tasks/{task_id}/artifacts`
    pub async fn list_task_artifacts(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::artifact::ListArtifactsResponse, ApiError> {
        self.get(&format!("/api/v1/tasks/{}/artifacts", urlenc(task_id)))
            .await
    }

    /// GET `/api/v1/artifacts/{artifact_id}/records`, filtered by key `prefix`
    /// and change time `since` when given.
    pub async fn list_artifact_records(
        &self,
        artifact_id: &str,
        prefix: Option<&str>,
        since: Option<&str>,
    ) -> Result<moatless_api_schemas::artifact::ListArtifactRecordsResponse, ApiError> {
        let params: Vec<String> = [("prefix", prefix), ("since", since)]
            .into_iter()
            .filter_map(|(name, value)| value.map(|v| format!("{name}={}", urlenc(v))))
            .collect();
        let query = if params.is_empty() {
            String::new()
        } else {
            format!("?{}", params.join("&"))
        };
        self.get(&format!(
            "/api/v1/artifacts/{}/records{query}",
            urlenc(artifact_id)
        ))
        .await
    }

    /// POST `/api/v1/artifacts/{artifact_id}/records`
    pub async fn write_artifact_record(
        &self,
        artifact_id: &str,
        req: &moatless_api_schemas::artifact::WriteArtifactRecordRequest,
    ) -> Result<moatless_api_schemas::artifact::ArtifactRecordResponse, ApiError> {
        self.post(
            &format!("/api/v1/artifacts/{}/records", urlenc(artifact_id)),
            req,
        )
        .await
    }
}
