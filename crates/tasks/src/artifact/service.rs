//! Artifact publishing and reading: validation, the Task access check, and the
//! reserve → upload → commit sequence that keeps a version immutable.

use super::error::ArtifactError;
use super::models::{Artifact, PublishRequest};
use super::operations::Artifacts;
use super::schemas::ArtifactResponse;
use crate::task::operations::TaskCommands;
use moatless_types::access_control::ScopeContext;

pub use moatless_api_schemas::artifact::MAX_PAGE_BYTES;
const MAX_TITLE_LEN: usize = 200;
const MAX_SLUG_LEN: usize = 63;

/// Publish `req` as the next version of its slug on `task_id`.
pub async fn publish(
    tasks: &TaskCommands,
    artifacts: &Artifacts,
    scope: &ScopeContext,
    task_id: &str,
    req: PublishRequest,
) -> Result<ArtifactResponse, ArtifactError> {
    let title = req.title.trim().to_string();
    if title.is_empty() || title.chars().count() > MAX_TITLE_LEN {
        return Err(ArtifactError::BadRequest(format!(
            "title must be 1-{MAX_TITLE_LEN} characters"
        )));
    }
    let slug = match req.slug.as_deref().map(str::trim) {
        Some(slug) if !slug.is_empty() => validate_slug(slug)?,
        _ => slug_from_title(&title),
    };
    check_page(req.file_name.as_deref(), &req.html)?;
    let task = tasks
        .find_by_id(task_id, scope)
        .await?
        .ok_or_else(|| ArtifactError::TaskNotFound(task_id.to_string()))?;
    if !scope.can_write(&task.scoped_resource()) {
        return Err(ArtifactError::Forbidden(task_id.to_string()));
    }
    if !artifacts.is_configured() {
        return Err(ArtifactError::StorageNotConfigured);
    }

    let reservation = artifacts
        .reserve_version(
            &uuid::Uuid::new_v4().to_string(),
            task_id,
            &slug,
            &title,
            &scope.user_id,
        )
        .await?;
    let artifact_id = reservation.artifact_id;
    let version = reservation.reserved_version;
    artifacts
        .put_page(task_id, &artifact_id, version, req.html)
        .await?;

    let row = match artifacts
        .commit_version(&artifact_id, version, &title)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            artifacts.delete_page(task_id, &artifact_id, version).await;
            return Err(ArtifactError::TaskNotFound(task_id.to_string()));
        }
        Err(e) => recover_commit(artifacts, task_id, &artifact_id, version, e).await?,
    };
    Ok(to_response(&row, artifacts.public_url(), version, &title))
}

/// After `commit_version` failed: the row when the commit landed anyway, since
/// the error can arrive after Postgres applied it. Deletes the page only when a
/// read-back shows `version` unpublished, and keeps it when that read fails too.
async fn recover_commit(
    artifacts: &Artifacts,
    task_id: &str,
    artifact_id: &str,
    version: i32,
    err: sqlx::Error,
) -> Result<Artifact, ArtifactError> {
    match artifacts.find_by_id(artifact_id).await {
        Ok(Some(row)) if row.published_versions.contains(&version) => Ok(row),
        Ok(Some(_)) => {
            artifacts.delete_page(task_id, artifact_id, version).await;
            Err(err.into())
        }
        Ok(None) => {
            artifacts.delete_page(task_id, artifact_id, version).await;
            Err(ArtifactError::TaskNotFound(task_id.to_string()))
        }
        Err(read_err) => {
            tracing::warn!(
                error = %read_err,
                %artifact_id,
                version,
                "Cannot tell whether an artifact version committed; keeping its page"
            );
            Err(err.into())
        }
    }
}

/// The Task's published Artifacts, most recently updated first.
pub async fn list(
    tasks: &TaskCommands,
    artifacts: &Artifacts,
    scope: &ScopeContext,
    task_id: &str,
) -> Result<Vec<ArtifactResponse>, ArtifactError> {
    tasks
        .find_by_id(task_id, scope)
        .await?
        .ok_or_else(|| ArtifactError::TaskNotFound(task_id.to_string()))?;
    let rows = artifacts.list_for_task(task_id).await?;
    Ok(rows
        .iter()
        .map(|row| to_response(row, artifacts.public_url(), row.latest_version, &row.title))
        .collect())
}

/// Path of the latest version's page, relative to the server root.
pub async fn latest_location(
    tasks: &TaskCommands,
    artifacts: &Artifacts,
    scope: &ScopeContext,
    artifact_id: &str,
) -> Result<String, ArtifactError> {
    let row = find_readable(tasks, artifacts, scope, artifact_id).await?;
    Ok(version_path(&row.artifact_id, row.latest_version))
}

/// The HTML page of a published version. `version` is the URL segment, as in
/// `v3`.
pub async fn read_page(
    tasks: &TaskCommands,
    artifacts: &Artifacts,
    scope: &ScopeContext,
    artifact_id: &str,
    version: &str,
) -> Result<Vec<u8>, ArtifactError> {
    let not_found = || ArtifactError::NotFound(format!("{artifact_id}/{version}"));
    let row = find_readable(tasks, artifacts, scope, artifact_id).await?;
    let version = parse_version(version)
        .filter(|v| row.published_versions.contains(v))
        .ok_or_else(not_found)?;
    artifacts
        .read_page(&row.task_id, &row.artifact_id, version)
        .await?
        .ok_or_else(not_found)
}

/// The Artifact, when it is published on a Task `scope` can read.
async fn find_readable(
    tasks: &TaskCommands,
    artifacts: &Artifacts,
    scope: &ScopeContext,
    artifact_id: &str,
) -> Result<Artifact, ArtifactError> {
    let not_found = || ArtifactError::NotFound(artifact_id.to_string());
    let row = artifacts
        .find_by_id(artifact_id)
        .await?
        .filter(|row| row.latest_version > 0)
        .ok_or_else(not_found)?;
    tasks
        .find_by_id(&row.task_id, scope)
        .await?
        .ok_or_else(not_found)?;
    Ok(row)
}

fn to_response(row: &Artifact, public_url: &str, version: i32, title: &str) -> ArtifactResponse {
    ArtifactResponse {
        artifact_id: row.artifact_id.clone(),
        task_id: row.task_id.clone(),
        slug: row.slug.clone(),
        title: title.to_string(),
        version,
        url: format!("{public_url}{}", version_path(&row.artifact_id, version)),
        latest_url: format!("{public_url}/api/v1/artifacts/{}", row.artifact_id),
        created_by: row.created_by.clone(),
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    }
}

fn version_path(artifact_id: &str, version: i32) -> String {
    format!("/api/v1/artifacts/{artifact_id}/v{version}")
}

fn parse_version(segment: &str) -> Option<i32> {
    segment
        .strip_prefix('v')
        .and_then(|n| n.parse::<i32>().ok())
        .filter(|n| *n > 0)
}

/// One non-empty page named `*.html` or `*.htm`, at most [`MAX_PAGE_BYTES`].
fn check_page(file_name: Option<&str>, html: &[u8]) -> Result<(), ArtifactError> {
    let Some(file_name) = file_name else {
        return Err(ArtifactError::BadRequest(
            "a `file` part with the HTML page is required".to_string(),
        ));
    };
    if !is_html_file_name(file_name) {
        return Err(ArtifactError::BadRequest(format!(
            "{file_name:?} is not an .html or .htm file"
        )));
    }
    if html.is_empty() {
        return Err(ArtifactError::BadRequest(format!("{file_name} is empty")));
    }
    if html.len() > MAX_PAGE_BYTES {
        return Err(ArtifactError::PayloadTooLarge(format!(
            "{file_name} exceeds {MAX_PAGE_BYTES} bytes"
        )));
    }
    Ok(())
}

fn is_html_file_name(file_name: &str) -> bool {
    std::path::Path::new(file_name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"))
}

fn validate_slug(slug: &str) -> Result<String, ArtifactError> {
    let valid = slug.len() <= MAX_SLUG_LEN
        && slug
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if valid {
        Ok(slug.to_string())
    } else {
        Err(ArtifactError::BadRequest(format!(
            "slug {slug:?} must be lowercase letters, digits and '-', starting with a letter or digit, at most {MAX_SLUG_LEN} characters"
        )))
    }
}

/// Lowercase ASCII alphanumerics of `title`, runs of anything else collapsed
/// to one `-`; `artifact` when nothing is left.
fn slug_from_title(title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.truncate(MAX_SLUG_LEN);
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "artifact".to_string()
    } else {
        slug.to_string()
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
