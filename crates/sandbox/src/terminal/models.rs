//! Inputs and observations owned by the terminal runtime.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TerminalSessionStatus {
    #[default]
    Starting,
    Running,
    Exited,
    Error,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalOpenRequest {
    pub thread_id: String,
    pub terminal_id: String,
    pub cwd: String,
    pub worktree_path: Option<String>,
    pub cols: u32,
    pub rows: u32,
    pub env: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalAttachRequest {
    pub thread_id: String,
    pub terminal_id: String,
    pub cwd: Option<String>,
    pub worktree_path: Option<String>,
    pub cols: u32,
    pub rows: u32,
    pub env: HashMap<String, String>,
    pub restart_if_not_running: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalWriteRequest {
    pub thread_id: String,
    pub terminal_id: String,
    pub data: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalResizeRequest {
    pub thread_id: String,
    pub terminal_id: String,
    pub cols: u32,
    pub rows: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalClearRequest {
    pub thread_id: String,
    pub terminal_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalRestartRequest {
    pub thread_id: String,
    pub terminal_id: String,
    pub cwd: String,
    pub worktree_path: Option<String>,
    pub cols: u32,
    pub rows: u32,
    pub env: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalCloseRequest {
    pub thread_id: String,
    pub terminal_id: Option<String>,
    pub delete_history: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalSessionSnapshot {
    pub thread_id: String,
    pub terminal_id: String,
    pub cwd: String,
    pub worktree_path: Option<String>,
    pub status: TerminalSessionStatus,
    pub pid: Option<u32>,
    pub history: String,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub label: String,
    pub updated_at: String,
    pub sequence: u64,
    pub started_new_process: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalStartedEvent {
    pub snapshot: Option<TerminalSessionSnapshot>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalOutputEvent {
    pub data: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalExitedEvent {
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalErrorEvent {
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalRestartedEvent {
    pub snapshot: Option<TerminalSessionSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    Started(TerminalStartedEvent),
    Output(TerminalOutputEvent),
    Exited(TerminalExitedEvent),
    Closed(bool),
    Error(TerminalErrorEvent),
    Cleared(bool),
    Restarted(TerminalRestartedEvent),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalAttachStreamEvent {
    pub thread_id: String,
    pub terminal_id: String,
    pub sequence: u64,
    pub event: Option<TerminalEvent>,
}
