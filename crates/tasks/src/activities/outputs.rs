//! Settled calls' outputs, held across reads of the same transcript window.
//!
//! A live thread re-reads its whole window on every move of its head, and all
//! but the newest calls' outputs are what the previous read already fetched.

use std::collections::HashMap;
use std::sync::Arc;

use moatless_core::db::DbPool;
use moatless_types::activity::{Activity, ActivityKind, ActivityStatus};
use serde_json::Value;

use super::dao;

/// How many bytes of output, counted as compact JSON, the cache holds.
const CAPACITY_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug)]
struct Cached {
    version: String,
    result: Option<Value>,
    typed_result: Option<Value>,
}

/// Outputs by activity id, each valid for the row version it was read at.
#[derive(Clone)]
pub(crate) struct OutputCache {
    entries: moka::future::Cache<String, Arc<Cached>>,
}

impl OutputCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: moka::future::Cache::builder()
                .max_capacity(CAPACITY_BYTES)
                .weigher(|id: &String, cached: &Arc<Cached>| {
                    let bytes = id.len()
                        + cached.version.len()
                        + cached.result.as_ref().map_or(0, encoded_len)
                        + cached.typed_result.as_ref().map_or(0, encoded_len);
                    u32::try_from(bytes).unwrap_or(u32::MAX)
                })
                .build(),
        }
    }

    /// `rows`, each `(activity, version)` as
    /// [`dao::for_task_since_without_output`] read it, with every settled
    /// call's output put back into its `payload`.
    ///
    /// An entry whose version no longer matches is read again: a row whose
    /// output was rewritten has moved its version with it.
    pub(crate) async fn restore(
        &self,
        pool: &DbPool,
        task_id: &str,
        rows: Vec<(Activity, String)>,
    ) -> Result<Vec<Activity>, sqlx::Error> {
        let mut found: HashMap<String, Arc<Cached>> = HashMap::new();
        let mut missing = Vec::new();
        for (activity, version) in &rows {
            if !has_output(activity) {
                continue;
            }
            match self.entries.get(&activity.activity_id).await {
                Some(cached) if cached.version == *version => {
                    found.insert(activity.activity_id.clone(), cached);
                }
                _ => missing.push(activity.activity_id.clone()),
            }
        }
        for output in dao::outputs(pool, task_id, &missing).await? {
            let cached = Arc::new(Cached {
                version: output.version,
                result: output.result,
                typed_result: output.typed_result,
            });
            self.entries
                .insert(output.activity_id.clone(), cached.clone())
                .await;
            found.insert(output.activity_id, cached);
        }
        Ok(rows
            .into_iter()
            .map(|(mut activity, _)| {
                if let (Some(cached), Some(payload)) = (
                    found.get(&activity.activity_id),
                    activity.payload.as_object_mut(),
                ) {
                    if let Some(result) = &cached.result {
                        payload.insert("result".into(), result.clone());
                    }
                    if let Some(typed) = &cached.typed_result {
                        payload.insert("typedResult".into(), typed.clone());
                    }
                }
                activity
            })
            .collect())
    }
}

/// Only a call that has come back has an output to restore.
fn has_output(activity: &Activity) -> bool {
    activity.kind == ActivityKind::ToolCall && activity.status != ActivityStatus::InProgress
}

fn encoded_len(value: &Value) -> usize {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    let _ = serde_json::to_writer(&mut counter, value);
    counter.0
}
