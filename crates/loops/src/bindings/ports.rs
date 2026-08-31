//! The Binding domain's implementations of the read seams it answers.
//!
//! An Adapter that only *asks about* Bindings reads [`BindingProbe`], published
//! by the Adapter kernel (`moatless_adapter_model::binding`); [`BindingDirectory`]
//! below answers it over the Binding store it holds. An Adapter that *drives* the
//! Binding lifecycle — `adapter-github-pr` and `adapter-linear`, which both
//! upsert through [`crate::bindings::runtime::upsert_binding_into`] as well as
//! read and write status — names [`TaskBindingDao`] itself: a narrow read port
//! beside a store it already holds is a second surface, not a seam.
//!
//! The Task band sits *below* this crate and reaches Binding through
//! `moatless_backend_core::binding::TaskBindings`, implemented at the bottom of
//! this file.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use moatless_adapter_model::binding::BindingProbe;
use moatless_adapter_model::models::TaskBinding;
use moatless_backend_core::binding::{TaskBindingSummary, TaskBindings};
use moatless_backend_core::store::StoreError;

use crate::bindings::dao::TaskBindingDao;
use crate::loops::dao::LoopDao;

/// The Binding store and the Loops that label its rows, behind the two seams a
/// crate that does not drive the Binding lifecycle reads: [`BindingProbe`] and
/// [`TaskBindings`].
pub struct BindingDirectory {
    bindings: Arc<dyn TaskBindingDao>,
    loops: Arc<LoopDao>,
}

impl BindingDirectory {
    pub fn new(bindings: Arc<dyn TaskBindingDao>, loops: Arc<LoopDao>) -> Self {
        Self { bindings, loops }
    }
}

#[async_trait]
impl BindingProbe for BindingDirectory {
    async fn is_bound(
        &self,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<bool, StoreError> {
        let rows = self
            .bindings
            .find_by_subject(adapter_kind, subject_kind, subject_key)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("read bindings on {adapter_kind}:{subject_key}"), e)
            })?;
        Ok(!rows.is_empty())
    }
}

#[async_trait]
impl TaskBindings for BindingDirectory {
    async fn summaries(&self, task_id: &str) -> Result<Vec<TaskBindingSummary>, StoreError> {
        let bindings = self
            .bindings
            .find_by_task_id(task_id)
            .await
            .map_err(|e| StoreError::with_source(format!("read bindings on task {task_id}"), e))?;

        let mut summaries = Vec::with_capacity(bindings.len());
        for binding in &bindings {
            // A Loop that cannot be read labels the Binding from itself rather
            // than dropping it: the Binding is what the caller asked about.
            let subscription = match binding.loop_id.as_deref() {
                Some(loop_id) => self
                    .loops
                    .find_subscription_by_id(loop_id)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            summaries.push(summarize(binding, subscription.as_ref()));
        }
        Ok(summaries)
    }

    async fn list_summaries(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, Vec<TaskBindingSummary>>, StoreError> {
        let rows = self
            .bindings
            .get_bindings_for_tasks(task_ids)
            .await
            .map_err(|e| StoreError::with_source("batch read task bindings", e))?;

        let mut out: HashMap<String, Vec<TaskBindingSummary>> = HashMap::new();
        for row in rows {
            out.entry(row.task_id).or_default().push(TaskBindingSummary {
                loop_id: row.loop_id,
                adapter_kind: moatless_adapter_model::models::parse_adapter_kind(&row.adapter_kind),
                source_id: row.source_id,
                source_name: row.source_name,
                thread_id: None,
                external_thread_id: None,
                subject_key: None,
                binding_id: None,
                origin: None,
                last_event_at: None,
            });
        }
        Ok(out)
    }

    async fn deactivate_for_task(&self, task_id: &str) -> Result<u64, StoreError> {
        self.bindings.deactivate_by_task(task_id).await.map_err(|e| {
            StoreError::with_source(format!("deactivate bindings on task {task_id}"), e)
        })
    }
}

/// One Binding as the API presents it, labelled by the Loop that owns it.
fn summarize(
    binding: &TaskBinding,
    subscription: Option<&moatless_adapter_model::models::SubscriptionMatch>,
) -> TaskBindingSummary {
    let source_id = subscription.map_or_else(
        || binding.subject_key.clone(),
        |subscription| subscription.source_matcher.clone(),
    );
    let external_thread_id = match binding.adapter_kind.as_str() {
        "github" => binding
            .destination_config
            .get("pr_number")
            .and_then(serde_json::Value::as_u64)
            .map(|n| n.to_string()),
        "slack" => binding
            .destination_config
            .get("thread_ts")
            .and_then(serde_json::Value::as_str)
            .map(String::from),
        "telegram" => binding
            .destination_config
            .get("message_id")
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string()),
        _ => None,
    };

    TaskBindingSummary {
        loop_id: subscription
            .map(|subscription| subscription.id.clone())
            .unwrap_or_default(),
        adapter_kind: binding.adapter_kind,
        source_id: source_id.clone(),
        source_name: Some(source_id),
        thread_id: None,
        external_thread_id,
        subject_key: Some(binding.subject_key.clone()),
        binding_id: Some(binding.id.clone()),
        origin: Some(binding.origin.clone()),
        last_event_at: Some(binding.last_event_at.to_rfc3339()),
    }
}
