//! `moat gh` — GitHub proxy for sandbox agents, one child per responsibility.
//!
//! [`dispatch`] classifies the invocation. [`typed`]'s subcommands and
//! [`stack`] route through `/api/v1/github/*` for task↔PR binding, the
//! `Moatless task:` footer and webhook dedup; [`api`] passes `gh api` through
//! and records what its POST created; [`pr_view`] serves a read real `gh`
//! answers with nothing; anything else reaches real `gh` through [`process`].
//! [`repo`] names the one repository every call addresses, and [`token`] the
//! credential it acts with.

mod api;
mod dispatch;
mod pr_view;
mod process;
mod repo;
mod stack;
mod token;
mod typed;

pub use dispatch::run;
pub use process::CommandExit;
pub(super) use process::gh_command_output;
pub(super) use token::github_token_for_gh;
