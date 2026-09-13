//! Task-event change-feed positions and persistence.
//!
//! Two readers over one log, and they want different things. The SSE fan-out
//! polls [`changes_after`] with a per-replica cursor and forwards `event`
//! without parsing it; the event handler claims batches off
//! [`claim_dispatch`]'s queue so the fleet runs each event's handlers once.

mod dao;
mod position;
pub use dao::{
    changes_after, claim_dispatch, current_position, dispatch_depth, prune_changes, record_event,
    release_dispatch,
};
pub use position::{DISPATCH_BATCH, EventChange, RECLAIM_AFTER, RETAINED_CHANGES};
