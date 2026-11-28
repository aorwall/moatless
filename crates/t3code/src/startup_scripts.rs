//! Starting the Scripts declared `run_on_start`, on the Workspace or the Task.
//!
//! A Sandbox becoming Ready is the other trigger for these Scripts, besides a
//! person pressing one; the Task band drives it, nothing here decides when.
//!
//! Every start goes through [`scripts::run`], not around it, so an automatic
//! start and a pressed one are identical, down to a restart attaching to an
//! already-running Script rather than starting a second one.

use std::sync::Arc;

use futures::{Stream, StreamExt};
use sandbox_proto::terminal_attach_stream_event::Event as ProtoEvent;
use sandbox_proto::{TerminalAttachRequest, TerminalAttachStreamEvent, TerminalSessionStatus};
use tracing::{debug, warn};

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

/// A startup Script this run started a process for, which
/// [`StartupScripts::watch_exit`] can follow.
pub struct StartedScript {
    pub name: String,
    pub terminal_id: String,
}

/// What [`StartupScripts::run_for_task`] did with each flagged Script.
#[derive(Default)]
pub struct StartupRun {
    pub started: Vec<StartedScript>,
    pub failures: Vec<StartupScriptFailure>,
}

/// A startup Script's process exiting non-zero on its own.
#[derive(Debug, PartialEq, Eq)]
pub struct ScriptExit {
    pub exit_code: i32,
    /// The last lines it printed, escape sequences stripped.
    pub tail: String,
}

/// How many lines of output a [`ScriptExit`] carries.
const EXIT_TAIL_LINES: usize = 15;

/// How much output [`failed_exit`] keeps while it waits, in bytes. A dev
/// server prints for hours; only the end is ever reported.
const OUTPUT_BUFFER_BYTES: usize = 16 * 1024;

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

    /// Start every Script declared `run_on_start`, the Task's own included,
    /// which brings a Task Script back after its pod was replaced.
    ///
    /// The `Result` error is the Workspace read failing; the vector is the
    /// Scripts that would not start, a failure never stopping the next one.
    pub async fn run_for_task(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<StartupRun, ScriptsError> {
        let resolved =
            scripts::resolve(&self.scripts, self.sandboxes.as_ref(), viewer, task_id).await?;
        // Only a log: this fires on every `sandbox.ready`, and most Tasks
        // declare nothing of their own, so a Script that was never there would
        // report a failure to the person on every start.
        if let Some(error) = &resolved.own_unread {
            warn!(
                task_id,
                %error,
                "A task's own run_on_start scripts were not read; only its workspace's started"
            );
        }

        let mut run = StartupRun::default();
        for script in resolved
            .scripts
            .iter()
            .map(|script| &script.config)
            .filter(|script| script.run_on_start)
        {
            let input = scripts::ScriptsRunInput {
                thread_id: task_id.to_string(),
                script_id: script.name.clone(),
            };
            match scripts::run(
                &self.scripts,
                self.reads.conversations(),
                self.sandboxes.as_ref(),
                &self.terminal,
                viewer,
                &input,
            )
            .await
            {
                Ok(started) if started["started"] == true => {
                    if let Some(terminal_id) = started["terminalId"].as_str() {
                        run.started.push(StartedScript {
                            name: script.name.clone(),
                            terminal_id: terminal_id.to_string(),
                        });
                    }
                }
                Ok(_) => {}
                Err(error) => run.failures.push(StartupScriptFailure {
                    name: script.name.clone(),
                    error,
                }),
            }
        }
        Ok(run)
    }

    /// Follow a Script [`Self::run_for_task`] started until its process exits,
    /// returning the exit when it is a failure.
    ///
    /// `None` for a zero exit, one by signal, and a stream that ends first —
    /// the Sandbox stopping, which is not the Script failing.
    pub async fn watch_exit(&self, task_id: &str, terminal_id: &str) -> Option<ScriptExit> {
        let client = match self.sandboxes.sandbox_client(task_id).await {
            Ok(client) => client,
            Err(error) => {
                debug!(task_id, terminal_id, %error, "No sandbox to watch a startup script in");
                return None;
            }
        };
        let request = TerminalAttachRequest {
            thread_id: task_id.to_string(),
            terminal_id: terminal_id.to_string(),
            ..Default::default()
        };
        match client.terminal_attach(request).await {
            Ok(stream) => failed_exit(stream).await,
            Err(error) => {
                debug!(task_id, terminal_id, %error, "A startup script could not be watched");
                None
            }
        }
    }
}

/// Read a Script's attach stream until it exits; see
/// [`StartupScripts::watch_exit`].
async fn failed_exit<E>(
    mut stream: impl Stream<Item = Result<TerminalAttachStreamEvent, E>> + Unpin,
) -> Option<ScriptExit> {
    let mut output = String::new();
    let exited = |code: Option<i32>, signal: Option<i32>, output: &str| match (code, signal) {
        (Some(code), None) if code != 0 => Some(ScriptExit {
            exit_code: code,
            tail: tail(output),
        }),
        _ => None,
    };
    while let Some(Ok(event)) = stream.next().await {
        match event.event? {
            ProtoEvent::Snapshot(snapshot)
            | ProtoEvent::Started(sandbox_proto::TerminalStartedEvent {
                snapshot: Some(snapshot),
            }) => {
                output = snapshot.history;
                if snapshot.status == TerminalSessionStatus::Exited as i32 {
                    return exited(snapshot.exit_code, snapshot.exit_signal, &output);
                }
            }
            ProtoEvent::Output(chunk) => {
                output.push_str(&chunk.data);
                if output.len() > OUTPUT_BUFFER_BYTES * 2 {
                    let mut cut = output.len() - OUTPUT_BUFFER_BYTES;
                    while !output.is_char_boundary(cut) {
                        cut += 1;
                    }
                    output.drain(..cut);
                }
            }
            ProtoEvent::Exited(exit) => {
                return exited(exit.exit_code, exit.exit_signal, &output);
            }
            ProtoEvent::Closed(_) | ProtoEvent::Error(_) => return None,
            _ => {}
        }
    }
    None
}

/// The last [`EXIT_TAIL_LINES`] non-blank lines of terminal output, with ANSI
/// escape sequences and carriage-return redraws removed.
fn tail(output: &str) -> String {
    let plain = strip_ansi(output);
    let lines: Vec<&str> = plain
        .split('\n')
        .map(|line| {
            let line = line.trim_end_matches('\r');
            line.rsplit('\r').next().unwrap_or(line).trim_end()
        })
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(EXIT_TAIL_LINES)..].join("\n")
}

/// Drop CSI (`ESC [ … final`) and OSC (`ESC ] … BEL|ESC \`) sequences.
fn strip_ansi(output: &str) -> String {
    let mut plain = String::with_capacity(output.len());
    let mut chars = output.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            plain.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    plain
}

#[cfg(test)]
mod tests {
    use sandbox_proto::{TerminalExitedEvent, TerminalOutputEvent, TerminalSessionSnapshot};

    use super::*;

    fn event(event: ProtoEvent) -> TerminalAttachStreamEvent {
        TerminalAttachStreamEvent {
            event: Some(event),
            ..Default::default()
        }
    }

    fn running() -> ProtoEvent {
        ProtoEvent::Snapshot(TerminalSessionSnapshot {
            status: TerminalSessionStatus::Running as i32,
            history: "Waiting for workspace setup to finish...\r\n".to_string(),
            ..Default::default()
        })
    }

    fn output(data: &str) -> ProtoEvent {
        ProtoEvent::Output(TerminalOutputEvent {
            data: data.to_string(),
        })
    }

    fn exit(code: i32, signal: Option<i32>) -> ProtoEvent {
        ProtoEvent::Exited(TerminalExitedEvent {
            exit_code: Some(code),
            exit_signal: signal,
        })
    }

    async fn watch(events: Vec<ProtoEvent>) -> Option<ScriptExit> {
        let events = events.into_iter().map(|e| Ok::<_, ()>(event(e)));
        failed_exit(futures::stream::iter(events)).await
    }

    /// The restart case: `node_modules` is gone and the dev server's binary
    /// with it.
    #[tokio::test]
    async fn a_script_exiting_non_zero_reports_its_code_and_last_output() {
        let exit = watch(vec![
            running(),
            output("$ vite --port 3000\r\n"),
            output("\u{1b}[31msh: 1: vite: not found\u{1b}[0m\r\n"),
            exit(127, None),
        ])
        .await;

        assert_eq!(
            exit,
            Some(ScriptExit {
                exit_code: 127,
                tail: "Waiting for workspace setup to finish...\n$ vite --port 3000\nsh: 1: vite: not found"
                    .to_string(),
            })
        );
    }

    #[tokio::test]
    async fn a_script_that_exited_before_the_watch_attached_is_reported() {
        let exit = watch(vec![ProtoEvent::Snapshot(TerminalSessionSnapshot {
            status: TerminalSessionStatus::Exited as i32,
            exit_code: Some(1),
            history: "boom\r\n".to_string(),
            ..Default::default()
        })])
        .await;

        assert_eq!(exit.map(|exit| exit.exit_code), Some(1));
    }

    #[tokio::test]
    async fn a_clean_exit_a_signal_or_a_lost_stream_is_not_a_failure() {
        assert_eq!(watch(vec![running(), exit(0, None)]).await, None);
        assert_eq!(watch(vec![running(), exit(143, Some(15))]).await, None);
        assert_eq!(watch(vec![running(), output("ready\r\n")]).await, None);
    }

    #[test]
    fn the_tail_keeps_the_last_lines_and_drops_redraws() {
        let mut output = String::new();
        for n in 0..30 {
            output.push_str("line ");
            output.push_str(&n.to_string());
            output.push_str("\r\n");
        }
        let tail = tail(&format!("{output}50%\r100%\r\n\u{1b}]0;title\u{7}"));
        assert_eq!(tail.lines().count(), EXIT_TAIL_LINES);
        assert!(tail.ends_with("line 29\n100%"), "{tail}");
    }
}
