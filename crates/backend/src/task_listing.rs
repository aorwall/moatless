//! Permanent backend facade for the Task listing feed.
//!
//! The feed itself lives in `moatless-backend-core`: it is written from more
//! than one band, so it sits below all of them. Its pruner is a supervised
//! timer and stays in `moatless-tasks`.

pub use moatless_backend_core::change_feed::listing::*;
pub use moatless_tasks::task_listing::spawn_change_pruner;
