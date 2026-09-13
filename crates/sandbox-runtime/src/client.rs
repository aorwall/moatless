//! Backend compatibility layer for the sandbox gRPC client.
//!
//! The client itself lives in `sandbox-proto`, with the protobuf service it
//! speaks. The backend keeps only API-schema conversions here.

pub use sandbox_proto::client::*;

use super::schemas::{
    BackgroundWatchJobInfo as SchemaBackgroundWatchJobInfo, ManagedCommand as SchemaManagedCommand,
    ManagedCommandState as SchemaManagedCommandState, ScheduledJobInfo as SchemaScheduledJobInfo,
    ScheduledJobKind, WakeupJobInfo as SchemaWakeupJobInfo,
};

// The schema types are defined in `moatless-api-schemas`; `sandbox_proto` is a
// separate crate, so these proto conversions cannot be inherent or `From` impls
// (orphan rule) and live here as free functions in the crate that names both.

/// Converts a proto `ManagedCommand` into the API schema.
///
/// An unrecognised state reads as `Running`: treating an unknown-but-in-flight
/// state as finished would let the reaper stop the pod underneath it.
pub fn managed_command_from_proto(proto: sandbox_proto::ManagedCommand) -> SchemaManagedCommand {
    let state = match proto.state() {
        sandbox_proto::ManagedCommandState::Exited => SchemaManagedCommandState::Exited,
        sandbox_proto::ManagedCommandState::TimedOut => SchemaManagedCommandState::TimedOut,
        sandbox_proto::ManagedCommandState::Killed => SchemaManagedCommandState::Killed,
        sandbox_proto::ManagedCommandState::Running
        | sandbox_proto::ManagedCommandState::Unspecified => SchemaManagedCommandState::Running,
    };
    SchemaManagedCommand {
        id: proto.id,
        command: proto.command,
        label: proto.label,
        pid: proto.pid,
        started_at_unix_ms: proto.started_at_unix_ms,
        timeout_secs: proto.timeout_secs,
        deadline_unix_ms: proto.deadline_unix_ms,
        state,
        exit_code: proto.exit_code,
        log_path: proto.log_path,
        finished_at_unix_ms: proto.finished_at_unix_ms,
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
                command_count: b.command_count,
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
