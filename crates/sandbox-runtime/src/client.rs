//! Backend compatibility layer for the sandbox gRPC client.
//!
//! The client itself lives in `sandbox-proto`, with the protobuf service it
//! speaks. The backend keeps only API-schema conversions here.

pub use sandbox_proto::client::*;

use super::schemas::{
    BackgroundProcess as SchemaBackgroundProcess,
    BackgroundWatchJobInfo as SchemaBackgroundWatchJobInfo,
    ScheduledJobInfo as SchemaScheduledJobInfo, ScheduledJobKind,
    WakeupJobInfo as SchemaWakeupJobInfo,
};

// The schema types are defined in `moatless-api-schemas`; `sandbox_proto` is a
// separate crate, so these proto conversions cannot be inherent or `From` impls
// (orphan rule) and live here as free functions in the crate that names both.

/// Convert a proto `BackgroundProcess` into the API schema.
pub fn background_process_from_proto(
    proto: sandbox_proto::BackgroundProcess,
) -> SchemaBackgroundProcess {
    SchemaBackgroundProcess {
        pid: proto.pid,
        command: proto.command,
    }
}

/// Convert a proto `ScheduledJobInfo` into the API schema.
///
/// Returns `None` for forward-compat unknown variants (proto `kind` is `None`)
/// so callers can silently drop them.
pub fn scheduled_job_info_from_proto(
    proto: sandbox_proto::ScheduledJobInfo,
) -> Option<SchemaScheduledJobInfo> {
    let sandbox_proto::ScheduledJobInfo {
        id,
        registered_at_unix_ms,
        kind,
    } = proto;
    let kind = match kind? {
        sandbox_proto::scheduled_job_info::Kind::Wakeup(w) => {
            ScheduledJobKind::Wakeup(SchemaWakeupJobInfo {
                reason: w.reason,
                prompt_preview: w.prompt_preview,
                wakeup_at_unix_ms: w.wakeup_at_unix_ms,
            })
        }
        sandbox_proto::scheduled_job_info::Kind::BackgroundWatch(b) => {
            ScheduledJobKind::BackgroundWatch(SchemaBackgroundWatchJobInfo {
                process_count: b.process_count,
                commands: b.commands,
            })
        }
    };
    Some(SchemaScheduledJobInfo {
        id,
        registered_at_unix_ms,
        kind,
    })
}
