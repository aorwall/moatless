//! Tmux-backed Claude query entry points; see the crate AGENTS.md for lifecycle rules.

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

pub use query::{query_tmux, query_tmux_streaming};
pub use crate::transcripts::{has_resumable_transcript, transcript_dir};
