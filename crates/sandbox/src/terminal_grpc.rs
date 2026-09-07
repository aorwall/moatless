//! Translate the terminal runtime contract at the gRPC boundary.

use crate::terminal::{self, TerminalError};

impl From<TerminalError> for tonic::Status {
    fn from(error: TerminalError) -> Self {
        match error {
            TerminalError::Missing(_)
            | TerminalError::InvalidDimension(_)
            | TerminalError::CwdOutsideWorkspace
            | TerminalError::CwdNotFound(_)
            | TerminalError::CwdNotDirectory(_) => {
                tonic::Status::invalid_argument(error.to_string())
            }
            TerminalError::NotFound => tonic::Status::not_found(error.to_string()),
            TerminalError::NotRunning => tonic::Status::failed_precondition(error.to_string()),
            TerminalError::Operation(_) => tonic::Status::internal(error.to_string()),
        }
    }
}

impl From<sandbox_proto::TerminalOpenRequest> for terminal::TerminalOpenRequest {
    fn from(value: sandbox_proto::TerminalOpenRequest) -> Self {
        Self {
            thread_id: value.thread_id,
            terminal_id: value.terminal_id,
            cwd: value.cwd,
            worktree_path: value.worktree_path,
            cols: value.cols,
            rows: value.rows,
            env: value.env,
        }
    }
}

impl From<sandbox_proto::TerminalAttachRequest> for terminal::TerminalAttachRequest {
    fn from(value: sandbox_proto::TerminalAttachRequest) -> Self {
        Self {
            thread_id: value.thread_id,
            terminal_id: value.terminal_id,
            cwd: value.cwd,
            worktree_path: value.worktree_path,
            cols: value.cols,
            rows: value.rows,
            env: value.env,
            restart_if_not_running: value.restart_if_not_running,
        }
    }
}

impl From<sandbox_proto::TerminalWriteRequest> for terminal::TerminalWriteRequest {
    fn from(value: sandbox_proto::TerminalWriteRequest) -> Self {
        Self {
            thread_id: value.thread_id,
            terminal_id: value.terminal_id,
            data: value.data,
        }
    }
}

impl From<sandbox_proto::TerminalResizeRequest> for terminal::TerminalResizeRequest {
    fn from(value: sandbox_proto::TerminalResizeRequest) -> Self {
        Self {
            thread_id: value.thread_id,
            terminal_id: value.terminal_id,
            cols: value.cols,
            rows: value.rows,
        }
    }
}

impl From<sandbox_proto::TerminalClearRequest> for terminal::TerminalClearRequest {
    fn from(value: sandbox_proto::TerminalClearRequest) -> Self {
        Self {
            thread_id: value.thread_id,
            terminal_id: value.terminal_id,
        }
    }
}

impl From<sandbox_proto::TerminalRestartRequest> for terminal::TerminalRestartRequest {
    fn from(value: sandbox_proto::TerminalRestartRequest) -> Self {
        Self {
            thread_id: value.thread_id,
            terminal_id: value.terminal_id,
            cwd: value.cwd,
            worktree_path: value.worktree_path,
            cols: value.cols,
            rows: value.rows,
            env: value.env,
        }
    }
}

impl From<sandbox_proto::TerminalCloseRequest> for terminal::TerminalCloseRequest {
    fn from(value: sandbox_proto::TerminalCloseRequest) -> Self {
        Self {
            thread_id: value.thread_id,
            terminal_id: value.terminal_id,
            delete_history: value.delete_history,
        }
    }
}

impl From<terminal::TerminalSessionSnapshot> for sandbox_proto::TerminalSessionSnapshot {
    fn from(value: terminal::TerminalSessionSnapshot) -> Self {
        Self {
            thread_id: value.thread_id,
            terminal_id: value.terminal_id,
            cwd: value.cwd,
            worktree_path: value.worktree_path,
            status: match value.status {
                terminal::TerminalSessionStatus::Starting => {
                    sandbox_proto::TerminalSessionStatus::Starting as i32
                }
                terminal::TerminalSessionStatus::Running => {
                    sandbox_proto::TerminalSessionStatus::Running as i32
                }
                terminal::TerminalSessionStatus::Exited => {
                    sandbox_proto::TerminalSessionStatus::Exited as i32
                }
                terminal::TerminalSessionStatus::Error => {
                    sandbox_proto::TerminalSessionStatus::Error as i32
                }
            },
            pid: value.pid,
            history: value.history,
            exit_code: value.exit_code,
            exit_signal: value.exit_signal,
            label: value.label,
            updated_at: value.updated_at,
            sequence: value.sequence,
            started_new_process: value.started_new_process,
        }
    }
}

impl From<terminal::TerminalStartedEvent> for sandbox_proto::TerminalStartedEvent {
    fn from(value: terminal::TerminalStartedEvent) -> Self {
        Self {
            snapshot: value.snapshot.map(Into::into),
        }
    }
}

impl From<terminal::TerminalOutputEvent> for sandbox_proto::TerminalOutputEvent {
    fn from(value: terminal::TerminalOutputEvent) -> Self {
        Self { data: value.data }
    }
}

impl From<terminal::TerminalExitedEvent> for sandbox_proto::TerminalExitedEvent {
    fn from(value: terminal::TerminalExitedEvent) -> Self {
        Self {
            exit_code: value.exit_code,
            exit_signal: value.exit_signal,
        }
    }
}

impl From<terminal::TerminalErrorEvent> for sandbox_proto::TerminalErrorEvent {
    fn from(value: terminal::TerminalErrorEvent) -> Self {
        Self {
            message: value.message,
        }
    }
}

impl From<terminal::TerminalRestartedEvent> for sandbox_proto::TerminalRestartedEvent {
    fn from(value: terminal::TerminalRestartedEvent) -> Self {
        Self {
            snapshot: value.snapshot.map(Into::into),
        }
    }
}

impl From<terminal::TerminalAttachStreamEvent> for sandbox_proto::TerminalAttachStreamEvent {
    fn from(value: terminal::TerminalAttachStreamEvent) -> Self {
        use sandbox_proto::terminal_attach_stream_event::Event;
        Self {
            thread_id: value.thread_id,
            terminal_id: value.terminal_id,
            sequence: value.sequence,
            event: value.event.map(|event| match event {
                terminal::TerminalEvent::Started(value) => Event::Started(value.into()),
                terminal::TerminalEvent::Output(value) => Event::Output(value.into()),
                terminal::TerminalEvent::Exited(value) => Event::Exited(value.into()),
                terminal::TerminalEvent::Error(value) => Event::Error(value.into()),
                terminal::TerminalEvent::Restarted(value) => Event::Restarted(value.into()),
                terminal::TerminalEvent::Closed(value) => Event::Closed(value),
                terminal::TerminalEvent::Cleared(value) => Event::Cleared(value),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_statuses_keep_the_wire_values() {
        for (status, expected) in [
            (terminal::TerminalSessionStatus::Starting, 0),
            (terminal::TerminalSessionStatus::Running, 1),
            (terminal::TerminalSessionStatus::Exited, 2),
            (terminal::TerminalSessionStatus::Error, 3),
        ] {
            let snapshot: sandbox_proto::TerminalSessionSnapshot =
                terminal::TerminalSessionSnapshot {
                    status,
                    started_new_process: Some(false),
                    ..Default::default()
                }
                .into();
            assert_eq!(snapshot.status, expected);
            assert_eq!(snapshot.started_new_process, Some(false));
        }
    }

    #[test]
    fn terminal_errors_keep_the_rpc_codes() {
        for (error, expected) in [
            (
                TerminalError::CwdOutsideWorkspace,
                tonic::Code::InvalidArgument,
            ),
            (TerminalError::NotFound, tonic::Code::NotFound),
            (TerminalError::NotRunning, tonic::Code::FailedPrecondition),
            (
                TerminalError::Operation("pty closed".into()),
                tonic::Code::Internal,
            ),
        ] {
            let message = error.to_string();
            let status: tonic::Status = error.into();
            assert_eq!(status.code(), expected);
            assert_eq!(status.message(), message);
        }
    }

    #[test]
    fn stream_conversion_preserves_session_identity_sequence_and_payload() {
        let snapshot = terminal::TerminalSessionSnapshot {
            thread_id: "task-a".into(),
            terminal_id: "shell-2".into(),
            cwd: "/workspace/repo".into(),
            worktree_path: Some("/workspace/tree".into()),
            status: terminal::TerminalSessionStatus::Exited,
            pid: Some(42),
            history: "last output".into(),
            exit_code: Some(7),
            exit_signal: Some(9),
            label: "Shell".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            sequence: 12,
            started_new_process: None,
        };
        let event: sandbox_proto::TerminalAttachStreamEvent = terminal::TerminalAttachStreamEvent {
            thread_id: snapshot.thread_id.clone(),
            terminal_id: snapshot.terminal_id.clone(),
            sequence: snapshot.sequence,
            event: Some(terminal::TerminalEvent::Restarted(
                terminal::TerminalRestartedEvent {
                    snapshot: Some(snapshot.clone()),
                },
            )),
        }
        .into();
        assert_eq!(event.thread_id, "task-a");
        assert_eq!(event.terminal_id, "shell-2");
        assert_eq!(event.sequence, 12);
        let Some(sandbox_proto::terminal_attach_stream_event::Event::Restarted(restarted)) =
            event.event
        else {
            panic!("expected restarted event");
        };
        let wire = restarted.snapshot.unwrap();
        assert_eq!(wire.cwd, snapshot.cwd);
        assert_eq!(wire.worktree_path, snapshot.worktree_path);
        assert_eq!(wire.pid, Some(42));
        assert_eq!(wire.history, "last output");
        assert_eq!(wire.exit_code, Some(7));
        assert_eq!(wire.exit_signal, Some(9));
        assert_eq!(wire.label, "Shell");
        assert_eq!(wire.updated_at, snapshot.updated_at);
        assert_eq!(wire.started_new_process, None);
    }
}
