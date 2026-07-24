//! Adapter, trigger, and binding API request/response schemas.
//!
//! Re-exported from the `moatless-api-schemas` crate.
//! Conversion functions live here since we can't define inherent impls on foreign types.

pub use moatless_api_schemas::adapters::*;
pub use moatless_api_schemas::bindings::*;
pub use moatless_api_schemas::loops::*;

use crate::adapters::common::parse_adapter_kind;

pub fn task_binding_summary_from_binding(
    binding: &crate::adapters::models::TaskBinding,
    rule: Option<&crate::adapters::models::SubscriptionMatch>,
) -> TaskBindingSummary {
    let source_id = rule.map_or_else(|| binding.subject_key.clone(), |r| r.source_matcher.clone());
    let external_thread_id = match binding.provider.as_str() {
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
        trigger_id: rule.map(|r| r.id.clone()).unwrap_or_default(),
        adapter_kind: parse_adapter_kind(&binding.provider),
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
