//! Git write actions on the primary checkout.
//!
//! The read-only git surface (status, diff, refs) lives under [`crate::files`];
//! this module owns the one write the T3 toolbar drives — a stacked action of
//! an optional branch, then a commit, a push and a pull request — and the
//! one-shot message generation it needs when the caller gave no commit message.

pub mod actions;
pub mod generate;
