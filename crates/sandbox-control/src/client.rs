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

impl SchemaBackgroundProcess {
    /// Convert a proto `BackgroundProcess` into the API schema.
    pub fn from_proto(proto: sandbox_proto::BackgroundProcess) -> Self {
        Self {
            pid: proto.pid,
            command: proto.command,
        }
    }
}

impl SchemaScheduledJobInfo {
    /// Convert a proto `ScheduledJobInfo` into the API schema.
    ///
    /// Returns `None` for forward-compat unknown variants (proto `kind` is
    /// `None`) so callers can silently drop them.
    pub fn from_proto(proto: sandbox_proto::ScheduledJobInfo) -> Option<Self> {
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
        Some(Self {
            id,
            registered_at_unix_ms,
            kind,
        })
    }
}
