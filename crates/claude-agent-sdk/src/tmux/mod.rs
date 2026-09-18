//! Tmux-backed Claude query entry points.
//!
//! The TUI lives behind a detached tmux session. Claude's hook system is the
//! readiness signal and the turn-completion signal:
//!
//! 1. `HookHarness::create` mkfifos
//!    `$TMPDIR/claude-sdk-tmux-<pid>-<uuid>/events.fifo` and writes a relay
//!    shell script.
//! 2. `--settings '<json>'` registers `SessionStart` and `Stop` hooks that
//!    pipe their payloads into the FIFO, one `"<event>\t<payload>\n"` line
//!    each.
//! 3. A `spawn_blocking` reader opens the FIFO `O_RDWR | O_NONBLOCK`, so it
//!    never sees EOF when the relay disconnects between events. It forwards
//!    each parsed line over an unbounded mpsc.
//! 4. `spawn_hook_listener` drains that channel and republishes a
//!    higher-level `HookState` on a `watch`: `Pending`, `SessionStarted` or
//!    `TurnEnded`.
//! 5. `startup::` awaits `SessionStarted` before `query::` pastes the prompt,
//!    under `init_timeout` (60s by default). No step sleeps for a fixed time.
//!    `startup_dialog_watcher` runs in parallel: it polls `tmux capture-pane`,
//!    strips CSI, and answers the trust and bypass dialogs (`trust`).
//! 6. `stop_fallback_loop` watches for a `Stop` event. Each one gives the
//!    tail loop up to 2s (40 × 50ms) to emit a `Result` from the JSONL
//!    `turn_duration` record. Past that it synthesizes `ResultSuccess` from
//!    the Stop payload's `last_assistant_message`.
//!
//! `Stop`'s `transcript_path` also drives JSONL discovery. That matters for
//! `--continue`, where claude appends to the existing transcript and no new
//! file appears. Snapshot-diff stays as a defensive fallback for a Stop
//! payload that omits the path.

mod args;
mod hooks;
mod session;
mod subagents;
mod tailer;
mod translator;
mod trust;
mod writer;

mod query;
mod startup;

pub use crate::transcripts::{has_resumable_transcript, transcript_dir};
pub use query::{query_tmux, query_tmux_streaming};
