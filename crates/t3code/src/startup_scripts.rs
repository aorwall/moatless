//! Starting the Scripts a Workspace declares `run_on_start`.
//!
//! A Sandbox that just became Ready has no dev server on it, no matter that the
//! Workspace declares one: a Script is run by a person pressing something. This
//! is the other trigger — the Sandbox becoming Ready — for the Scripts that
//! asked for it. The Task band drives it; nothing here decides when.
//!
//! Every start goes through [`scripts::run`], not around it, so an automatic
//! start and a pressed one are the same thing: the port is published, the
//! command runs in the terminal `script-<name>`, the preview tab opens, and the
//! setup gate holds it until the Workspace's setup commands have finished. It
//! also means a restart that finds the Script already running attaches to it.
//!
//! This is backend work rather than sandbox work because publishing the port is
//! [`SandboxPortPublisher`](moatless_ports::sandbox::SandboxPortPublisher) — a
//! Service patch the sandbox cannot make for itself.

use std::sync::Arc;

use serde_json::json;

use crate::reads::T3Reads;
use crate::scripts::{self, ScriptsError, ScriptsState};
use crate::terminal::TerminalHub;
use moatless_types::access_control::ScopeContext;
use sandbox_proto::client::SandboxConnector;

/// The handles a startup run needs, sharing the router's own.
///
/// `TerminalHub` is in-process state, so this holds the *same* hub the RPC
/// surface serves terminal reads from — a second one would host sessions
/// nothing else can see. Built in [`compose`](crate::compose) alongside the
/// router for that reason.
#[derive(Clone)]
pub struct StartupScripts {
    scripts: ScriptsState,
    reads: T3Reads,
    sandboxes: Arc<dyn SandboxConnector>,
    terminal: TerminalHub,
}

/// A Script that was flagged to start, and why it did not.
pub struct StartupScriptFailure {
    pub name: String,
    pub error: ScriptsError,
}

impl StartupScripts {
    pub(crate) fn new(
        scripts: ScriptsState,
        reads: T3Reads,
        sandboxes: Arc<dyn SandboxConnector>,
        terminal: TerminalHub,
    ) -> Self {
        Self {
            scripts,
            reads,
            sandboxes,
            terminal,
        }
    }

    /// Start every Script the Task's Workspace declares `run_on_start`.
    ///
    /// The error is the Workspace read failing, which says nothing about any
    /// one Script; the vector is the Scripts that would not start, in
    /// declaration order. A Script that fails does not stop the next one — they
    /// are separate processes, and the caller reports each failure on its own.
    pub async fn run_for_task(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<Vec<StartupScriptFailure>, ScriptsError> {
        let scripts = self
            .scripts
            .workspaces
            .scripts_for_task(viewer, task_id)
            .await?
            .ok_or(ScriptsError::NotFound)?;

        let mut failures = Vec::new();
        for script in scripts.iter().filter(|script| script.run_on_start) {
            let payload = json!({ "threadId": task_id, "scriptId": script.name });
            if let Err(error) = scripts::run(
                &self.scripts,
                self.reads.conversations(),
                self.sandboxes.as_ref(),
                &self.terminal,
                viewer,
                &payload,
            )
            .await
            {
                failures.push(StartupScriptFailure {
                    name: script.name.clone(),
                    error,
                });
            }
        }
        Ok(failures)
    }
}
