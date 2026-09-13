//! Starting the Scripts a Workspace declares `run_on_start`.
//!
//! A Sandbox becoming Ready is the other trigger for these Scripts, besides a
//! person pressing one; the Task band drives it, nothing here decides when.
//!
//! Every start goes through [`scripts::run`], not around it, so an automatic
//! start and a pressed one are identical, down to a restart attaching to an
//! already-running Script rather than starting a second one.

use std::sync::Arc;

use serde_json::json;

use crate::reads::T3Reads;
use crate::scripts::{self, ScriptsError, ScriptsState};
use crate::terminal::TerminalHub;
use moatless_types::access_control::ScopeContext;
use sandbox_proto::client::SandboxConnector;

/// The handles a startup run needs, sharing the router's own.
///
/// `TerminalHub` is in-process state: this holds the *same* hub the RPC surface
/// serves terminal reads from, or a second one would host sessions nothing else
/// can see.
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
    /// The `Result` error is the Workspace read failing; the vector is the
    /// Scripts that would not start. A failed Script does not stop the next
    /// one.
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
