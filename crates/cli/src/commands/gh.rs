//! `moat gh` — GitHub proxy for sandbox agents, one child per responsibility.
//!
//! [`typed`] and [`stack`] route through `/api/v1/github/*` for task↔PR
//! binding, the `Moatless task:` footer and dedup; [`task_footer`] keeps that
//! footer on an edited body. [`dispatch`] says where each call goes.

mod api;
mod branch_write;
mod dispatch;
mod notified;
mod pr_edit;
mod pr_followups;
mod pr_view;
mod process;
mod reply_target;
mod repo;
mod run_logs;
mod stack;
mod task_footer;
mod token;
mod typed;

pub use dispatch::run;
pub(super) use process::gh_command_output;
pub(super) use token::github_token_for_gh;
