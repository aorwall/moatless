//! What a record write may carry: its key, its data and the collection's
//! name. Every collection, an Artifact's own included, checks writes here.

use moatless_api_schemas::artifact::{MAX_RECORD_BYTES, WriteArtifactRecordRequest};
use moatless_api_schemas::collection::MAX_COLLECTION_NAME_LEN;

/// Most records one collection holds, list items included; a write that
/// would add one more is refused, so a writer stuck in a loop cannot fill the
/// table.
pub const MAX_RECORDS: i64 = 10_000;
const MAX_KEY_LEN: usize = 200;

/// Why a write's key, data or collection name is refused.
#[derive(Debug, thiserror::Error)]
pub enum InvalidRecord {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    TooLarge(String),
}

/// `req` once its key and data are valid.
pub fn checked(
    req: WriteArtifactRecordRequest,
) -> Result<WriteArtifactRecordRequest, InvalidRecord> {
    let (key, data) = match &req {
        WriteArtifactRecordRequest::Save { key, data } => (key.as_str(), Some(data)),
        // Room for the `/` and the 36-character id `item_key` appends.
        WriteArtifactRecordRequest::Append { list, data } => {
            valid_key(list, MAX_KEY_LEN - 37)?;
            return valid_data(data).map(|()| req);
        }
        WriteArtifactRecordRequest::Delete { key } => (key.as_str(), None),
    };
    valid_key(key, MAX_KEY_LEN)?;
    if let Some(data) = data {
        valid_data(data)?;
    }
    Ok(req)
}

/// The key an append to `list` saves under: a v7 UUID sorts by creation time,
/// so the list's keys sort in the order its items were appended.
pub fn item_key(list: &str) -> String {
    format!("{list}/{}", uuid::Uuid::now_v7())
}

/// A collection name: one key segment of at most [`MAX_COLLECTION_NAME_LEN`]
/// bytes.
pub fn valid_name(name: &str) -> Result<(), InvalidRecord> {
    if !name.contains('/') && valid_key(name, MAX_COLLECTION_NAME_LEN).is_ok() {
        return Ok(());
    }
    Err(InvalidRecord::Invalid(format!(
        "collection name {name:?} must be at most {MAX_COLLECTION_NAME_LEN} letters, digits, \
         '-', '_', '.' or ':'"
    )))
}

/// `/`-separated segments of letters, digits, `-`, `_`, `.` or `:`, none empty.
fn valid_key(key: &str, max_len: usize) -> Result<(), InvalidRecord> {
    let valid = key.len() <= max_len
        && key.split('/').all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
        });
    if valid {
        return Ok(());
    }
    Err(InvalidRecord::Invalid(format!(
        "key {key:?} must be at most {max_len} characters of '/'-separated segments, each \
         letters, digits, '-', '_', '.' or ':'"
    )))
}

fn valid_data(data: &serde_json::Value) -> Result<(), InvalidRecord> {
    if data.is_null() {
        return Err(InvalidRecord::Invalid("data is required".to_string()));
    }
    if serde_json::to_vec(data).map_or(0, |v| v.len()) > MAX_RECORD_BYTES {
        return Err(InvalidRecord::TooLarge(format!(
            "a record is at most {MAX_RECORD_BYTES} bytes"
        )));
    }
    Ok(())
}
