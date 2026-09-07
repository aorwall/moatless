//! Compatibility exports for the owning `outbound_events_dao` persistence module.

pub use super::outbound_events_dao::{
    has_any, ids_for_task, record, task_for_any, task_for_any_from_source,
};
