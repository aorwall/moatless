//! Adapter metric recording and stable routing reason labels.

pub mod inbound_reason;
mod recording;

pub use recording::{
    BINDING_INBOUND_TOTAL, describe_adapter_metrics, record_api_call, record_inbound,
    record_webhook,
};
