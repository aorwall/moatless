//! Adapter, Loop, Subscription, and Binding API request/response schemas.
//!
//! Re-exported from the `moatless-api-schemas` crate.
//! Conversion functions live here since we can't define inherent impls on foreign types.

pub use moatless_api_schemas::adapters::*;
pub use moatless_api_schemas::bindings::*;
pub use moatless_api_schemas::loops::*;

use crate::adapters::common::parse_adapter_kind;

pub fn task_binding_summary_from_binding(
    binding: &crate::adapters::models::TaskBinding,
    subscription: Option<&crate::adapters::models::SubscriptionMatch>,
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
        adapter_kind: parse_adapter_kind(&binding.adapter_kind),
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
