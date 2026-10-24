//! Memory operations: which scopes a Task reaches, the access check, and the
//! validation every entry point shares.

use chrono::{DateTime, Utc};

use super::error::MemoryError;
use super::models::{
    ClaimOutcome, ListFilter, MemoryEntryRow, MemoryEventRow, PutEntry, PutOutcome, ScopeRef,
    TtlChange, Writer, decode_op, decode_scope,
};
use super::operations::Memories;
use super::schemas::{
    AppendMemoryRequest, ClaimMemoryRequest, MAX_BODY_BYTES, MAX_MEMORY_KEY_LEN, MAX_SUMMARY_CHARS,
    MemoryEntryResponse, MemoryEventResponse, MemoryLogResponse, MemorySummaryResponse,
    PutMemoryRequest,
};
use crate::task::models::Task;
use crate::task::operations::TaskCommands;
use moatless_types::access_control::{Scope, ScopeContext, ScopedResource};
use moatless_types::memory::{MemoryScope, is_valid_key};

const MAX_TAGS: usize = 20;
const MAX_TAG_LEN: usize = 64;
const MAX_CITATIONS: usize = 20;
const MAX_CITATION_LEN: usize = 500;
const MAX_APPEND_BYTES: usize = 4 * 1024;
/// Ten years: anything longer is a typo for `noExpiry`.
const MAX_SECONDS: i64 = 10 * 365 * 24 * 60 * 60;
const DEFAULT_LIST_LIMIT: i64 = 100;
const MAX_LIST_LIMIT: i64 = 500;
const DEFAULT_SEARCH_LIMIT: i64 = 20;
const MAX_SEARCH_LIMIT: i64 = 100;
const DEFAULT_LOG_LIMIT: i64 = 20;
const MAX_LOG_LIMIT: i64 = 200;
/// Most entries one `prefix*` get returns.
const MAX_PREFIX_GET: i64 = 50;

/// `requested` as `task` reaches it, or when the request names none the
/// Task's Loop, else the Task itself. A request reaches one scope only,
/// deliberately: a read merged across scopes made two scopes' `state` keys
/// collide.
fn scope_ref(
    ctx: &ScopeContext,
    task: &Task,
    requested: Option<MemoryScope>,
) -> Result<ScopeRef, MemoryError> {
    let requested = requested.unwrap_or_else(|| default_scope(task));
    // The owner's own memory: a caller who can merely read someone else's
    // Task does not reach it.
    if requested == MemoryScope::User && owner(task).is_some_and(|o| o != ctx.user_id) {
        return Err(MemoryError::Forbidden(format!(
            "only the owner of task {} reaches its user memory",
            task.task_id
        )));
    }
    let id = match requested {
        MemoryScope::Task => Some(task.task_id.as_str()),
        MemoryScope::Loop => task.loop_id.as_deref(),
        MemoryScope::Workspace => task.workspace_id.as_deref(),
        MemoryScope::Repository => task.repository_id.as_deref(),
        MemoryScope::User => owner(task),
        MemoryScope::Global => Some(""),
    };
    id.filter(|id| !id.is_empty() || requested == MemoryScope::Global)
        .map(|id| ScopeRef {
            scope: requested,
            id: id.to_string(),
        })
        .ok_or_else(|| {
            MemoryError::BadRequest(format!("task {} has no {requested} scope", task.task_id))
        })
}

/// The Loop scope of `task`, which a Loop's `memory.preload` reads.
pub fn loop_scope(task: &Task) -> Option<ScopeRef> {
    task.loop_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .map(|id| ScopeRef {
            scope: MemoryScope::Loop,
            id: id.to_string(),
        })
}

/// The private scope an unscoped request reaches, deliberately rather than
/// `global`: an unscoped put on a private repository must not publish its
/// text to every user.
fn default_scope(task: &Task) -> MemoryScope {
    if loop_scope(task).is_some() {
        MemoryScope::Loop
    } else {
        MemoryScope::Task
    }
}

fn owner(task: &Task) -> Option<&str> {
    task.owner_user_id.as_deref().or(task.created_by.as_deref())
}

/// The scope a write lands in, after its write check.
async fn write_scope(
    memories: &Memories,
    ctx: &ScopeContext,
    task: &Task,
    requested: Option<MemoryScope>,
) -> Result<ScopeRef, MemoryError> {
    let scope = scope_ref(ctx, task, requested)?;
    check_scope_write(memories, ctx, task, &scope).await?;
    Ok(scope)
}

/// The Task, when `ctx` can read it.
async fn readable_task(
    tasks: &TaskCommands,
    ctx: &ScopeContext,
    task_id: &str,
) -> Result<Task, MemoryError> {
    tasks
        .find_by_id(task_id, ctx)
        .await?
        .ok_or_else(|| MemoryError::TaskNotFound(task_id.to_string()))
}

/// The Task and its writer, when `ctx` can change the Task.
async fn writable_task(
    tasks: &TaskCommands,
    ctx: &ScopeContext,
    task_id: &str,
) -> Result<(Task, Writer), MemoryError> {
    let task = readable_task(tasks, ctx, task_id).await?;
    if !ctx.can_write(&task.scoped_resource()) {
        return Err(MemoryError::Forbidden(format!(
            "cannot write memory for task {task_id}"
        )));
    }
    let writer = Writer {
        task_id: task.task_id.clone(),
        loop_id: task.loop_id.clone(),
        user_id: ctx.user_id.clone(),
    };
    Ok((task, writer))
}

/// Refuse a write to a scope other users' Tasks read unless `ctx` may write
/// what it belongs to: the Workspace or repository itself, and for `global`
/// an admin or a global Task. Writing the Task is not enough: on a shared
/// repository it would let any user put text in another user's memory.
async fn check_scope_write(
    memories: &Memories,
    ctx: &ScopeContext,
    task: &Task,
    scope: &ScopeRef,
) -> Result<(), MemoryError> {
    let allowed = match scope.scope {
        MemoryScope::Task | MemoryScope::Loop | MemoryScope::User => true,
        MemoryScope::Global => ctx.is_admin || task.scope == Scope::Global,
        MemoryScope::Workspace => memories
            .workspaces
            .describe(ctx, &scope.id)
            .await?
            .is_some_and(|w| ctx.can_write(&ScopedResource::new(w.scope, w.owner_user_id))),
        MemoryScope::Repository => memories
            .repositories
            .describe(ctx, &scope.id)
            .await?
            .is_some_and(|r| ctx.can_write(&ScopedResource::new(r.scope, r.owner_user_id))),
    };
    if allowed {
        Ok(())
    } else {
        Err(MemoryError::Forbidden(format!(
            "cannot write {} memory for task {}",
            scope.scope, task.task_id
        )))
    }
}

/// Create or update an entry.
pub async fn put(
    tasks: &TaskCommands,
    memories: &Memories,
    ctx: &ScopeContext,
    task_id: &str,
    req: PutMemoryRequest,
) -> Result<MemoryEntryResponse, MemoryError> {
    validate_key(&req.key)?;
    let summary = req.summary.as_deref().map(validate_summary).transpose()?;
    if let Some(body) = &req.body {
        check_size("body", body.len())?;
    }
    if let Some(data) = &req.data {
        check_size("data", data.to_string().len())?;
    }
    let tags = req.tags.map(validate_tags).transpose()?;
    let citations = req.citations.map(validate_citations).transpose()?;
    let ttl = match (req.ttl_seconds, req.no_expiry) {
        (Some(_), true) => {
            return Err(MemoryError::BadRequest(
                "ttlSeconds and noExpiry cannot both be set".to_string(),
            ));
        }
        (Some(seconds), false) => TtlChange::Set(validate_seconds("ttlSeconds", seconds)?),
        (None, true) => TtlChange::NoExpiry,
        (None, false) => TtlChange::Keep,
    };
    let stale_after_seconds = req
        .stale_after_seconds
        .map(|s| validate_seconds("staleAfterSeconds", s))
        .transpose()?;
    if req.if_version.is_some_and(|v| v < 0) {
        return Err(MemoryError::BadRequest(
            "ifVersion must be 0 or more".to_string(),
        ));
    }

    let (task, writer) = writable_task(tasks, ctx, task_id).await?;
    let scope = write_scope(memories, ctx, &task, req.scope).await?;
    let key = req.key;
    let entry = PutEntry {
        scope,
        key: key.clone(),
        summary,
        body: req.body,
        data: req.data,
        tags,
        citations,
        ttl,
        stale_after_seconds,
        if_version: req.if_version,
        writer,
    };
    match memories.store.put(&entry).await? {
        PutOutcome::Written(row) => Ok(entry_response(&row)),
        PutOutcome::VersionMismatch { current: Some(v) } => Err(MemoryError::Conflict(format!(
            "{key} is at version {v}, not {}",
            entry.if_version.unwrap_or_default()
        ))),
        PutOutcome::VersionMismatch { current: None } => Err(MemoryError::Conflict(format!(
            "{key} does not exist, so it is not at version {}",
            entry.if_version.unwrap_or_default()
        ))),
        PutOutcome::SummaryRequired => Err(MemoryError::BadRequest(format!(
            "{key} is a new entry and needs a summary"
        ))),
    }
}

/// Whole entries for `key`, or for every key starting with it when it ends
/// in `*`, and record that the Task read them.
pub async fn get(
    tasks: &TaskCommands,
    memories: &Memories,
    ctx: &ScopeContext,
    task_id: &str,
    key: &str,
    scope: Option<MemoryScope>,
) -> Result<Vec<MemoryEntryResponse>, MemoryError> {
    let task = readable_task(tasks, ctx, task_id).await?;
    let scopes = [scope_ref(ctx, &task, scope)?];
    let rows = if let Some(prefix) = key.strip_suffix('*') {
        if !prefix.is_empty() {
            validate_key(prefix)?;
        }
        let filter = ListFilter {
            prefix: Some(prefix.to_string()).filter(|p| !p.is_empty()),
            limit: MAX_PREFIX_GET,
            ..ListFilter::default()
        };
        memories.store.list(&scopes, &filter).await?
    } else {
        validate_key(key)?;
        memories.store.find(&scopes, key).await?
    };
    if rows.is_empty() {
        return Err(MemoryError::NotFound(key.to_string()));
    }
    let reads: Vec<(String, i32)> = rows
        .iter()
        .map(|r| (r.entry_id.clone(), r.version))
        .collect();
    // A failed read log must not cost the agent the entry it asked for.
    if let Err(e) = memories.store.record_reads(task_id, &reads).await {
        tracing::warn!(error = %e, task_id, "Failed to record memory reads");
    }
    Ok(rows.iter().map(entry_response).collect())
}

/// What a listing is narrowed by.
#[derive(Debug, Clone, Default)]
pub struct ListParams {
    pub scope: Option<MemoryScope>,
    pub prefix: Option<String>,
    pub tag: Option<String>,
    pub updated_since_seconds: Option<i64>,
    pub limit: Option<i64>,
}

/// Summaries of the entries the Task reaches, newest first.
pub async fn list(
    tasks: &TaskCommands,
    memories: &Memories,
    ctx: &ScopeContext,
    task_id: &str,
    params: ListParams,
) -> Result<Vec<MemorySummaryResponse>, MemoryError> {
    let task = readable_task(tasks, ctx, task_id).await?;
    let scopes = [scope_ref(ctx, &task, params.scope)?];
    let updated_since = params
        .updated_since_seconds
        .map(|s| validate_seconds("updatedSinceSeconds", s).map(|s| since(Utc::now(), s)))
        .transpose()?;
    let filter = ListFilter {
        prefix: params.prefix.filter(|p| !p.is_empty()),
        tag: params
            .tag
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty()),
        updated_since,
        limit: clamp_limit(params.limit, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT),
    };
    let rows = memories.store.list(&scopes, &filter).await?;
    Ok(rows.iter().map(summary_response).collect())
}

/// Summaries of the entries matching `query`, best match first.
pub async fn search(
    tasks: &TaskCommands,
    memories: &Memories,
    ctx: &ScopeContext,
    task_id: &str,
    query: &str,
    scope: Option<MemoryScope>,
    limit: Option<i64>,
) -> Result<Vec<MemorySummaryResponse>, MemoryError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(MemoryError::BadRequest("q must not be empty".to_string()));
    }
    let task = readable_task(tasks, ctx, task_id).await?;
    let scopes = [scope_ref(ctx, &task, scope)?];
    let limit = clamp_limit(limit, DEFAULT_SEARCH_LIMIT, MAX_SEARCH_LIMIT);
    let rows = memories.store.search(&scopes, query, limit).await?;
    Ok(rows.iter().map(summary_response).collect())
}

/// The entry `key` names for a write, after the write access check.
async fn writable_entry(
    tasks: &TaskCommands,
    memories: &Memories,
    ctx: &ScopeContext,
    task_id: &str,
    key: &str,
    scope: Option<MemoryScope>,
) -> Result<(MemoryEntryRow, Writer), MemoryError> {
    validate_key(key)?;
    let (task, writer) = writable_task(tasks, ctx, task_id).await?;
    let scopes = [scope_ref(ctx, &task, scope)?];
    check_scope_write(memories, ctx, &task, &scopes[0]).await?;
    let row = memories
        .store
        .find(&scopes, key)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| MemoryError::NotFound(key.to_string()))?;
    Ok((row, writer))
}

/// Add a dated line to an entry's history.
pub async fn append(
    tasks: &TaskCommands,
    memories: &Memories,
    ctx: &ScopeContext,
    task_id: &str,
    req: AppendMemoryRequest,
) -> Result<MemoryEntryResponse, MemoryError> {
    let text = req.text.trim();
    if text.is_empty() || text.len() > MAX_APPEND_BYTES {
        return Err(MemoryError::BadRequest(format!(
            "text must be 1-{MAX_APPEND_BYTES} bytes"
        )));
    }
    let (row, writer) = writable_entry(tasks, memories, ctx, task_id, &req.key, req.scope).await?;
    let row = memories
        .store
        .append(&row.entry_id, text, &writer)
        .await?
        .ok_or_else(|| MemoryError::NotFound(req.key.clone()))?;
    Ok(entry_response(&row))
}

/// Record that the entry's citations still hold, renewing its expiry.
pub async fn verify(
    tasks: &TaskCommands,
    memories: &Memories,
    ctx: &ScopeContext,
    task_id: &str,
    key: &str,
    scope: Option<MemoryScope>,
) -> Result<MemoryEntryResponse, MemoryError> {
    let (row, writer) = writable_entry(tasks, memories, ctx, task_id, key, scope).await?;
    let row = memories
        .store
        .verify(&row.entry_id, &writer)
        .await?
        .ok_or_else(|| MemoryError::NotFound(key.to_string()))?;
    Ok(entry_response(&row))
}

/// Claim the entry for the Task; 409 while another open Task holds it.
pub async fn claim(
    tasks: &TaskCommands,
    memories: &Memories,
    ctx: &ScopeContext,
    task_id: &str,
    req: ClaimMemoryRequest,
) -> Result<MemoryEntryResponse, MemoryError> {
    let ttl = validate_seconds("ttlSeconds", req.ttl_seconds)?;
    let (row, writer) = writable_entry(tasks, memories, ctx, task_id, &req.key, req.scope).await?;
    match memories.store.claim(&row.entry_id, ttl, &writer).await? {
        Some(ClaimOutcome::Claimed(row)) => Ok(entry_response(&row)),
        Some(ClaimOutcome::HeldBy(holder)) => Err(MemoryError::Conflict(format!(
            "{} is claimed by task {holder}",
            req.key
        ))),
        None => Err(MemoryError::NotFound(req.key)),
    }
}

/// Drop the Task's claim on the entry.
pub async fn release(
    tasks: &TaskCommands,
    memories: &Memories,
    ctx: &ScopeContext,
    task_id: &str,
    key: &str,
    scope: Option<MemoryScope>,
) -> Result<(), MemoryError> {
    let (row, writer) = writable_entry(tasks, memories, ctx, task_id, key, scope).await?;
    if memories.store.release(&row.entry_id, &writer).await? {
        Ok(())
    } else {
        Err(MemoryError::Conflict(format!(
            "task {task_id} holds no claim on {key}"
        )))
    }
}

/// Delete the entry and its history.
pub async fn delete(
    tasks: &TaskCommands,
    memories: &Memories,
    ctx: &ScopeContext,
    task_id: &str,
    key: &str,
    scope: Option<MemoryScope>,
) -> Result<(), MemoryError> {
    let (row, _) = writable_entry(tasks, memories, ctx, task_id, key, scope).await?;
    if memories.store.delete(&row.entry_id).await? {
        Ok(())
    } else {
        Err(MemoryError::NotFound(key.to_string()))
    }
}

/// The entry's history, newest first.
pub async fn log(
    tasks: &TaskCommands,
    memories: &Memories,
    ctx: &ScopeContext,
    task_id: &str,
    key: &str,
    scope: Option<MemoryScope>,
    limit: Option<i64>,
) -> Result<MemoryLogResponse, MemoryError> {
    validate_key(key)?;
    let task = readable_task(tasks, ctx, task_id).await?;
    let scopes = [scope_ref(ctx, &task, scope)?];
    let row = memories
        .store
        .find(&scopes, key)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| MemoryError::NotFound(key.to_string()))?;
    let limit = clamp_limit(limit, DEFAULT_LOG_LIMIT, MAX_LOG_LIMIT);
    let events = memories.store.log(&row.entry_id, limit).await?;
    Ok(MemoryLogResponse {
        key: row.key.clone(),
        scope: decode_scope(&row).unwrap_or(MemoryScope::Global),
        items: events.iter().filter_map(event_response).collect(),
    })
}

fn since(now: DateTime<Utc>, seconds: i64) -> DateTime<Utc> {
    now - chrono::Duration::seconds(seconds)
}

fn clamp_limit(limit: Option<i64>, default: i64, max: i64) -> i64 {
    limit.unwrap_or(default).clamp(1, max)
}

/// ASCII letters, digits and `:._/-`, starting with a letter or digit, at
/// most [`MAX_MEMORY_KEY_LEN`] bytes. `*` is excluded because it marks a prefix.
pub(crate) fn validate_key(key: &str) -> Result<(), MemoryError> {
    if is_valid_key(key) {
        Ok(())
    } else {
        Err(MemoryError::BadRequest(format!(
            "key {key:?} must be ASCII letters, digits and ':._/-', starting with a letter or \
             digit, at most {MAX_MEMORY_KEY_LEN} bytes"
        )))
    }
}

fn validate_summary(summary: &str) -> Result<String, MemoryError> {
    let summary = summary.trim();
    if summary.is_empty() || summary.chars().count() > MAX_SUMMARY_CHARS || summary.contains('\n') {
        return Err(MemoryError::BadRequest(format!(
            "summary must be one line of 1-{MAX_SUMMARY_CHARS} characters"
        )));
    }
    Ok(summary.to_string())
}

fn check_size(field: &str, len: usize) -> Result<(), MemoryError> {
    if len > MAX_BODY_BYTES {
        return Err(MemoryError::BadRequest(format!(
            "{field} exceeds {MAX_BODY_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_tags(tags: Vec<String>) -> Result<Vec<String>, MemoryError> {
    if tags.len() > MAX_TAGS
        || tags
            .iter()
            .any(|t| t.trim().is_empty() || t.len() > MAX_TAG_LEN)
    {
        return Err(MemoryError::BadRequest(format!(
            "at most {MAX_TAGS} tags of 1-{MAX_TAG_LEN} bytes"
        )));
    }
    // Lowercased because search matches tags as stored lexemes, while
    // `websearch_to_tsquery` lowercases the query.
    Ok(tags.into_iter().map(|t| t.trim().to_lowercase()).collect())
}

fn validate_citations(citations: Vec<String>) -> Result<Vec<String>, MemoryError> {
    if citations.len() > MAX_CITATIONS
        || citations
            .iter()
            .any(|c| c.trim().is_empty() || c.len() > MAX_CITATION_LEN)
    {
        return Err(MemoryError::BadRequest(format!(
            "at most {MAX_CITATIONS} citations of 1-{MAX_CITATION_LEN} bytes"
        )));
    }
    Ok(citations
        .into_iter()
        .map(|c| c.trim().to_string())
        .collect())
}

fn validate_seconds(field: &str, seconds: i64) -> Result<i64, MemoryError> {
    if seconds <= 0 || seconds > MAX_SECONDS {
        return Err(MemoryError::BadRequest(format!(
            "{field} must be 1-{MAX_SECONDS} seconds"
        )));
    }
    Ok(seconds)
}

fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339()
}

pub(crate) fn entry_response(row: &MemoryEntryRow) -> MemoryEntryResponse {
    MemoryEntryResponse {
        key: row.key.clone(),
        scope: decode_scope(row).unwrap_or(MemoryScope::Global),
        summary: row.summary.clone(),
        body: row.body.clone(),
        data: row.data.clone(),
        tags: row.tags.clone(),
        citations: row.citations.clone(),
        version: row.version,
        stale: row.stale,
        expires_at: row.expires_at.map(rfc3339),
        verified_at: row.verified_at.map(rfc3339),
        claim_expires_at: row
            .claimed_by_task_id
            .as_ref()
            .and(row.claim_expires_at)
            .map(rfc3339),
        claimed_by_task_id: row.claimed_by_task_id.clone(),
        updated_by_task_id: row.updated_by_task_id.clone(),
        created_by_task_id: row.created_by_task_id.clone(),
        created_by_loop_id: row.created_by_loop_id.clone(),
        created_by_user_id: row.created_by_user_id.clone(),
        created_at: rfc3339(row.created_at),
        updated_at: rfc3339(row.updated_at),
    }
}

pub(crate) fn summary_response(row: &MemoryEntryRow) -> MemorySummaryResponse {
    MemorySummaryResponse {
        key: row.key.clone(),
        scope: decode_scope(row).unwrap_or(MemoryScope::Global),
        summary: row.summary.clone(),
        tags: row.tags.clone(),
        version: row.version,
        stale: row.stale,
        claimed_by_task_id: row.claimed_by_task_id.clone(),
        updated_at: rfc3339(row.updated_at),
    }
}

fn event_response(row: &MemoryEventRow) -> Option<MemoryEventResponse> {
    Some(MemoryEventResponse {
        op: decode_op(row)?,
        version: row.version,
        text: row.text.clone(),
        task_id: row.task_id.clone(),
        created_at: rfc3339(row.created_at),
    })
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
