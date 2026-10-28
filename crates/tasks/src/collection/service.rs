//! The agent's reads and writes of a scoped record collection: which scope
//! the Task reaches, the access check, and the write itself.

use moatless_api_schemas::artifact::{RecordWriter, WriteArtifactRecordRequest};
use moatless_api_schemas::collection::{CollectionRecordResponse, CollectionResponse, MemoryScope};
use moatless_types::access_control::ScopeContext;

use super::error::CollectionError;
use super::keys::{MAX_RECORDS, checked, item_key, valid_name};
use super::models::{CollectionRow, Record};
use super::operations::Records;
use crate::scope_reach::{ScopeRef, check_scope_write, scope_ref};
use crate::task::models::Task;
use crate::task::operations::TaskCommands;

/// What a store error names the records by.
const WHAT: &str = "records";

/// Which records a list returns: keys starting with `prefix`, changed after
/// `since` (RFC 3339), each when given.
#[derive(Debug, Default, Clone, Copy)]
pub struct RecordFilter<'a> {
    pub prefix: Option<&'a str>,
    pub since: Option<&'a str>,
}

/// Save, append or delete a record in the collection `name` of the scope
/// `task_id` reaches, creating the collection on its first write.
pub async fn write(
    tasks: &TaskCommands,
    records: &Records,
    ctx: &ScopeContext,
    task_id: &str,
    name: &str,
    scope: Option<MemoryScope>,
    req: WriteArtifactRecordRequest,
) -> Result<CollectionRecordResponse, CollectionError> {
    valid_name(name)?;
    let req = checked(req)?;
    let task = readable_task(tasks, ctx, task_id).await?;
    if !ctx.can_write(&task.scoped_resource()) {
        return Err(CollectionError::Forbidden(format!(
            "cannot write records for task {task_id}"
        )));
    }
    let scope = scope_ref(ctx, &task, scope, WHAT)?;
    check_scope_write(records.owners(), ctx, &task, &scope, WHAT).await?;
    let (key, data) = match req {
        WriteArtifactRecordRequest::Save { key, data } => (key, data),
        WriteArtifactRecordRequest::Append { list, data } => (item_key(&list), data),
        // A delete never creates the collection it names.
        WriteArtifactRecordRequest::Delete { key } => {
            let record = match find(records, &scope, name).await? {
                Some(c) => records.store.delete_record(&c.collection_id, &key).await?,
                None => None,
            };
            let record = record
                .ok_or_else(|| CollectionError::NotFound(format!("record {key:?} in {name}")))?;
            return Ok(to_response(&record, name, scope.scope));
        }
    };
    let collection = records
        .store
        .ensure_collection(
            &uuid::Uuid::new_v4().to_string(),
            scope.scope.as_str(),
            &scope.id,
            name,
            task_id,
            &ctx.user_id,
        )
        .await?;
    under_cap(records, &collection, &key).await?;
    let record = records
        .store
        .save_record(
            &collection.collection_id,
            &key,
            &data,
            &ctx.user_id,
            RecordWriter::Agent,
        )
        .await?;
    Ok(to_response(&record, name, scope.scope))
}

/// The records of the collection `name` in the scope `task_id` reaches,
/// oldest change first; none when the collection does not exist yet.
pub async fn list(
    tasks: &TaskCommands,
    records: &Records,
    ctx: &ScopeContext,
    task_id: &str,
    name: &str,
    scope: Option<MemoryScope>,
    filter: RecordFilter<'_>,
) -> Result<Vec<CollectionRecordResponse>, CollectionError> {
    valid_name(name)?;
    let since = parse_since(filter.since)?;
    let task = readable_task(tasks, ctx, task_id).await?;
    let scope = scope_ref(ctx, &task, scope, WHAT)?;
    let Some(collection) = find(records, &scope, name).await? else {
        return Ok(Vec::new());
    };
    let rows = records
        .store
        .list_records(
            &collection.collection_id,
            since,
            filter.prefix.map(str::to_string),
        )
        .await?;
    Ok(rows
        .iter()
        .map(|record| to_response(record, name, scope.scope))
        .collect())
}

/// The collections in the scope `task_id` reaches, by name.
pub async fn list_collections(
    tasks: &TaskCommands,
    records: &Records,
    ctx: &ScopeContext,
    task_id: &str,
    scope: Option<MemoryScope>,
) -> Result<Vec<CollectionResponse>, CollectionError> {
    let task = readable_task(tasks, ctx, task_id).await?;
    let scope = scope_ref(ctx, &task, scope, WHAT)?;
    let rows = records
        .store
        .list_collections(scope.scope.as_str(), &scope.id)
        .await?;
    Ok(rows
        .iter()
        .map(|row| CollectionResponse {
            name: row.name.clone(),
            scope: scope.scope,
            record_count: row.record_count,
            last_updated_at: row.last_updated_at.map(|t| t.to_rfc3339()),
            created_at: row.created_at.to_rfc3339(),
        })
        .collect())
}

/// The collection `name` in `scope`, if any record was ever written to it.
pub(crate) async fn find(
    records: &Records,
    scope: &ScopeRef,
    name: &str,
) -> Result<Option<CollectionRow>, sqlx::Error> {
    records
        .store
        .find_collection(scope.scope.as_str(), &scope.id, name)
        .await
}

async fn readable_task(
    tasks: &TaskCommands,
    ctx: &ScopeContext,
    task_id: &str,
) -> Result<Task, CollectionError> {
    tasks
        .find_by_id(task_id, ctx)
        .await?
        .ok_or_else(|| CollectionError::TaskNotFound(task_id.to_string()))
}

async fn under_cap(
    records: &Records,
    collection: &CollectionRow,
    key: &str,
) -> Result<(), CollectionError> {
    if records.has_room(&collection.collection_id, key).await? {
        return Ok(());
    }
    Err(CollectionError::BadRequest(format!(
        "collection {} already holds {MAX_RECORDS} records; delete some or save under an existing key",
        collection.name
    )))
}

/// `since` as a time, refusing one that is not RFC 3339.
pub(crate) fn parse_since(
    since: Option<&str>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, CollectionError> {
    since
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|t| t.with_timezone(&chrono::Utc))
                .map_err(|_| {
                    CollectionError::BadRequest(format!("since {s:?} is not an RFC 3339 time"))
                })
        })
        .transpose()
}

pub(crate) fn to_response(
    record: &Record,
    name: &str,
    scope: MemoryScope,
) -> CollectionRecordResponse {
    CollectionRecordResponse {
        collection: name.to_string(),
        scope,
        key: record.key.clone(),
        data: record.data.clone(),
        submitted_by: record.submitted_by.clone(),
        writer: record.writer,
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
