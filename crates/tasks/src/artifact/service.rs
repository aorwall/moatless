//! Artifact publishing and reading: validation, the Task access check, and the
//! reserve → upload → commit sequence that keeps a version immutable.

use super::error::ArtifactError;
use super::models::{Artifact, AttachedCollection, DEFAULT_INLINE_HEIGHT, PublishRequest};
use super::operations::Artifacts;
use super::rules::RecordRules;
use super::schemas::ArtifactResponse;
use crate::collection::keys::valid_name;
use crate::collection::service::find;
use crate::scope_reach::{ScopeError, check_scope_write, scope_ref};
use crate::task::models::Task;
use crate::task::operations::TaskCommands;
use moatless_types::access_control::ScopeContext;
use moatless_types::activity::ArtifactShown;
use moatless_types::memory::MemoryScope;
use moatless_types::task::Visibility;

pub use moatless_api_schemas::artifact::MAX_PAGE_BYTES;
const MAX_TITLE_LEN: usize = 200;
const MAX_SLUG_LEN: usize = 63;
/// The frame heights T3 clamps an HTML render to.
const MIN_INLINE_HEIGHT: i32 = 80;
const MAX_INLINE_HEIGHT: i32 = 2000;
/// Most scoped collections one publish lets its page read.
const MAX_ATTACHED_COLLECTIONS: usize = 8;

/// Publish `req` as the next version of its slug on `task_id`.
pub async fn publish(
    tasks: &TaskCommands,
    artifacts: &Artifacts,
    scope: &ScopeContext,
    task_id: &str,
    mut req: PublishRequest,
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
    if req.data.notify_on_data && !req.data.accepts_data {
        return Err(ArtifactError::BadRequest(
            "notify needs data: only a page that accepts data can notify".to_string(),
        ));
    }
    if req.data.rules.is_some() && !req.data.accepts_data {
        return Err(ArtifactError::BadRequest(
            "rules need data: only a page that accepts data has records to rule".to_string(),
        ));
    }
    if req.data.rules.is_some() && req.data.notify_on_data {
        return Err(ArtifactError::BadRequest(
            "rules replace notify: name the writes that notify in the rules' notify lists"
                .to_string(),
        ));
    }
    let task = tasks
        .find_by_id(task_id, scope)
        .await?
        .ok_or_else(|| ArtifactError::TaskNotFound(task_id.to_string()))?;
    if !scope.can_write(&task.scoped_resource(), Some(task.visibility)) {
        return Err(ArtifactError::Forbidden(task_id.to_string()));
    }
    if !artifacts.is_configured() {
        return Err(ArtifactError::StorageNotConfigured);
    }
    req.data.collections = attach(artifacts, scope, &task, &req.collections).await?;

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

    let shown = req.inline.map(|inline| ArtifactShown {
        artifact_id: artifact_id.clone(),
        version,
        title: title.clone(),
        height: inline
            .height
            .unwrap_or(DEFAULT_INLINE_HEIGHT)
            .clamp(MIN_INLINE_HEIGHT, MAX_INLINE_HEIGHT),
    });
    let row = match artifacts
        .commit_version(
            &artifact_id,
            version,
            &title,
            &req.data,
            req.visibility,
            shown,
        )
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

/// Resolve each `<scope>:<name>` in `specs` against `task`'s scopes, creating
/// a collection nothing has written yet so the page can wait for its first
/// record.
async fn attach(
    artifacts: &Artifacts,
    ctx: &ScopeContext,
    task: &Task,
    specs: &[String],
) -> Result<Vec<AttachedCollection>, ArtifactError> {
    if specs.len() > MAX_ATTACHED_COLLECTIONS {
        return Err(ArtifactError::BadRequest(format!(
            "a page may read at most {MAX_ATTACHED_COLLECTIONS} collections"
        )));
    }
    let mut attached: Vec<AttachedCollection> = Vec::with_capacity(specs.len());
    for spec in specs {
        let (scope, name) = spec
            .split_once(':')
            .and_then(|(scope, name)| Some((MemoryScope::try_from(scope).ok()?, name)))
            .ok_or_else(|| {
                ArtifactError::BadRequest(format!(
                    "collection {spec:?} must be <scope>:<name>, with scope one of task, loop, \
                     workspace, repository, user or global"
                ))
            })?;
        valid_name(name)?;
        if attached.iter().any(|a| a.name == name) {
            return Err(ArtifactError::BadRequest(format!(
                "collection name {name:?} is attached twice; a page reads collections by name"
            )));
        }
        let reached = scope_ref(ctx, task, Some(scope), "records")
            .map_err(|e| ArtifactError::BadRequest(e.to_string()))?;
        let records = artifacts.records();
        // Only a caller who may write the scope creates a collection in it; a
        // reader-only publisher attaches one that already exists.
        let row = match check_scope_write(records.owners(), ctx, task, &reached, "records").await {
            Ok(()) => {
                records
                    .store
                    .ensure_collection(
                        &uuid::Uuid::new_v4().to_string(),
                        scope.as_str(),
                        &reached.id,
                        name,
                        &task.task_id,
                        &ctx.user_id,
                    )
                    .await?
            }
            Err(ScopeError::Store(e)) => return Err(e.into()),
            Err(denied) => find(records, &reached, name).await?.ok_or_else(|| {
                ArtifactError::BadRequest(format!(
                    "{denied}, and no collection {spec} exists to read"
                ))
            })?,
        };
        attached.push(AttachedCollection {
            scope,
            scope_id: reached.id,
            name: name.to_string(),
            collection_id: row.collection_id,
        });
    }
    Ok(attached)
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

/// Set who may read a published Artifact's pages and records, as a user who
/// can change its Task.
pub async fn set_visibility(
    tasks: &TaskCommands,
    artifacts: &Artifacts,
    scope: &ScopeContext,
    artifact_id: &str,
    visibility: Visibility,
) -> Result<ArtifactResponse, ArtifactError> {
    let not_found = || ArtifactError::NotFound(artifact_id.to_string());
    let row = artifacts
        .find_by_id(artifact_id)
        .await?
        .filter(|row| row.latest_version > 0)
        .ok_or_else(not_found)?;
    // A public page's reader still may not learn of its Task: not found, the
    // same answer as for an Artifact that does not exist.
    let task = tasks
        .find_by_id(&row.task_id, scope)
        .await?
        .ok_or_else(not_found)?;
    if !scope.can_write(&task.scoped_resource(), Some(task.visibility)) {
        return Err(ArtifactError::Forbidden(row.task_id));
    }
    let row = artifacts
        .set_visibility(artifact_id, visibility)
        .await?
        .ok_or_else(not_found)?;
    Ok(to_response(
        &row,
        artifacts.public_url(),
        row.latest_version,
        &row.title,
    ))
}

/// A published version's HTML page as it was uploaded.
#[derive(Debug)]
pub struct Page {
    pub html: Vec<u8>,
    pub artifact_id: String,
    pub title: String,
    /// Whether the page is served with a page token and the record script.
    /// The latest publish decides, whichever version is served.
    pub accepts_data: bool,
    /// Whether the version was shown inline, and so is served with T3's
    /// bootstrap injected.
    pub inline: bool,
    /// The latest publish's record rules, which the page script exposes.
    pub record_rules: Option<RecordRules>,
    /// The scoped collections the latest publish lets the page read.
    pub collections: Vec<AttachedCollection>,
}

/// The HTML page of a published version. `version` is the URL segment, as in
/// `v3`; `None` reads the latest version.
pub async fn read_page(
    tasks: &TaskCommands,
    artifacts: &Artifacts,
    scope: &ScopeContext,
    artifact_id: &str,
    version: Option<&str>,
) -> Result<Page, ArtifactError> {
    let (row, version) = find_version(tasks, artifacts, scope, artifact_id, version).await?;
    let html = artifacts
        .read_page(&row.task_id, &row.artifact_id, version)
        .await?
        .ok_or_else(|| ArtifactError::NotFound(format!("{artifact_id}/v{version}")))?;
    Ok(Page {
        html,
        artifact_id: row.artifact_id,
        title: row.title,
        accepts_data: row.accepts_data,
        inline: row.inline_versions.contains(&version),
        record_rules: row.record_rules,
        collections: row.collections,
    })
}

/// The readable Artifact and the published version `version` names, the
/// latest when `None`.
async fn find_version(
    tasks: &TaskCommands,
    artifacts: &Artifacts,
    scope: &ScopeContext,
    artifact_id: &str,
    version: Option<&str>,
) -> Result<(Artifact, i32), ArtifactError> {
    let row = find_readable(tasks, artifacts, scope, artifact_id).await?;
    let Some(segment) = version else {
        let latest = row.latest_version;
        return Ok((row, latest));
    };
    let version = parse_version(segment)
        .filter(|v| row.published_versions.contains(v))
        .ok_or_else(|| ArtifactError::NotFound(format!("{artifact_id}/{segment}")))?;
    Ok((row, version))
}

/// The Artifact, when it is published and either public or on a Task `scope`
/// can read.
pub(crate) async fn find_readable(
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
    if row.visibility == Visibility::Public {
        return Ok(row);
    }
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
        accepts_data: row.accepts_data,
        notify_on_data: row.notify_on_data,
        visibility: row.visibility,
        record_rules: row.record_rules.as_ref().and_then(|rules| {
            serde_json::to_value(rules)
                .inspect_err(|error| {
                    tracing::error!(%error, "Failed to encode an artifact's record rules");
                })
                .ok()
        }),
        collections: row
            .collections
            .iter()
            .map(|c| format!("{}:{}", c.scope, c.name))
            .collect(),
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
