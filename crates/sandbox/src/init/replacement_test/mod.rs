//! Sandbox-replacement lifecycle tests.
//!
//! On replacement, `run_init` restores Session state before Ready, resolves
//! interrupted-Run vs pending-Message precedence while dispatch is gated, then
//! opens the gate for the one authoritative dispatch (`open_for_work`).
//!
//! Tests drive a real `CoordinatorService` over a recording `Harness` and read
//! back the accepted `Input`, proving harness acceptance over enqueue shape.

mod backend;
mod support;

mod job_prompts;
mod joined_member;
mod lost_commands;
mod member_dispatch;
mod precedence;
mod readiness;
mod restore;
mod wake_notes;
