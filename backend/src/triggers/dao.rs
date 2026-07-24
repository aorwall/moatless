//! Persistence for unified triggers.
//!
//! The row types and queries were consolidated into [`crate::triggers::store`]
//! (`LoopStore`). This module now only re-exports the shared read shapes so
//! existing importers keep compiling; a later ticket merges the module tree.

pub use crate::triggers::store::{
    AdapterEventTriggerSourceRow, ScheduleTriggerSourceRow, TriggerListFilter, TriggerRecord,
    TriggerRow, TriggerRuleRow, TriggerSourceRow,
};
