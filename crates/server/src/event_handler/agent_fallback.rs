//! Re-running a Task's turn on another agent when its provider will not serve
//! it, in the order `AGENT_FALLBACK_ORDER` sets.
//!
//! The Task switches harness in place and its turn's input is queued again, so
//! the fallback keeps the Task's workspace, branch, Bindings and conversation.

use moatless_core::config::AgentFallbackOrder;
use moatless_tasks::agent_event::AgentEvent;
use moatless_tasks::task::error::TaskError;
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task::switch::{SwitchCredentials, SwitchNote, SwitchOutcome};
use moatless_types::access_control::ScopeContext;
use moatless_types::agent::{AgentProfile, Harness};
use moatless_types::model::ClaudeModel;

use std::sync::Arc;

use moatless_identity::ports::HarnessTokens;
use moatless_identity::secret::service::SecretService;
use moatless_loops::loops::operations::Loops;
use moatless_ports::sandbox::SandboxWaker;
use moatless_tasks::credential_sync::CredentialSync;
use moatless_tasks::message::send::MessageSendService;
use moatless_tasks::task::operations::TaskCommands;
use moatless_tasks::task::switch::TaskSwitchService;

/// What a fallback reads and writes, projected out of `AppState`.
pub struct FallbackPorts {
    pub hub_available: bool,
    pub order: AgentFallbackOrder,
    pub tasks: Arc<TaskCommands>,
    pub loops: Arc<Loops>,
    pub task_switch: Arc<TaskSwitchService>,
    pub message_send: Arc<MessageSendService>,
    pub sandbox_waker: Arc<dyn SandboxWaker>,
    pub secrets: Arc<SecretService>,
    pub harness_credentials: Arc<HarnessTokens>,
    pub credential_sync: Arc<CredentialSync>,
}

/// The `agent.error` reasons that mean the provider will not serve the turn,
/// each published only once the sandbox's retries are spent. A safety refusal
/// is `invalid_input` and must never be here: it is not for the other provider.
const FALLBACK_REASONS: &[&str] = &[
    "usage_limit",
    "rate_limit",
    "provider_unavailable",
    "connectivity",
];

fn falls_back_on(reason: Option<&str>) -> bool {
    reason.is_some_and(|reason| FALLBACK_REASONS.contains(&reason))
}

/// Whether a failed turn falls back, and to which harnesses, in order.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Fallback<'a> {
    To(&'a [Harness]),
    Skip(&'static str),
}

/// The catalog model a fallback onto `harness` runs; `None` is the harness's
/// default. Claude Code runs Opus rather than the profile's unresolved
/// default, so a fallback turn gets the strongest model.
fn fallback_model(harness: Harness) -> Option<&'static str> {
    match harness {
        Harness::ClaudeCode => Some(ClaudeModel::Opus.alias()),
        _ => None,
    }
}

/// What decides a fallback, read from the event, the Task and its Loop.
#[derive(Debug)]
pub(crate) struct FallbackFacts<'a> {
    pub reason: Option<&'a str>,
    /// The profile the failed turn ran; `None` for one the catalog lacks.
    pub failed_profile: Option<AgentProfile>,
    /// The Task's Loop's setting; `None` for a Task no Loop created, which
    /// falls back.
    pub loop_enabled: Option<bool>,
    /// Whether the failed turn was itself a fallback; `None` when it
    /// delivered no input to re-run.
    pub turn_was_fallback: Option<bool>,
}

pub(crate) fn decide<'a>(facts: &FallbackFacts<'_>, order: &'a AgentFallbackOrder) -> Fallback<'a> {
    if !falls_back_on(facts.reason) {
        return Fallback::Skip("the provider did not refuse the turn");
    }
    if facts.loop_enabled == Some(false) {
        return Fallback::Skip("the Loop turned fallback off");
    }
    match facts.turn_was_fallback {
        None => return Fallback::Skip("the turn delivered no input"),
        // Both agents are limited; a second switch would only ping-pong.
        Some(true) => return Fallback::Skip("the turn was already a fallback"),
        Some(false) => {}
    }
    let Some(failed) = facts.failed_profile.map(AgentProfile::harness) else {
        return Fallback::Skip("the failed profile is not in the catalog");
    };
    match order.targets(failed) {
        [] => Fallback::Skip("AGENT_FALLBACK_ORDER names no fallback for the harness"),
        targets => Fallback::To(targets),
    }
}

/// Handle `agent.error`: re-run the turn on another harness when [`decide`] says so.
///
/// The Task switches to the first target that accepts it and its failed turn
/// is queued again. Every failure is logged and left as the error the turn
/// already reported.
pub async fn fall_back_to_other_agent(state: &FallbackPorts, event: &AgentEvent) {
    let reason = event.payload.get("reason").and_then(|v| v.as_str());
    if !falls_back_on(reason) {
        return;
    }
    let task_id = event.task_id.as_str();
    let Some(turn_number) = event.turn_number else {
        tracing::info!(task_id, "Agent fallback skipped: the error names no turn");
        return;
    };
    // reason: agent-lifecycle event handler runs as background system with no caller identity
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();
    let task = match state.tasks.find_by_id(task_id, &sys_ctx).await {
        Ok(Some(task)) if task.status != TaskStatus::Closed => task,
        Ok(_) => return,
        Err(error) => {
            tracing::warn!(task_id, %error, "Agent fallback could not read the task");
            return;
        }
    };
    let loop_enabled = match task.loop_id.as_deref() {
        Some(loop_id) => match state.loops.fallback_to_other_agent(loop_id).await {
            Ok(enabled) => enabled,
            Err(error) => {
                tracing::warn!(task_id, loop_id, %error, "Agent fallback could not read the loop");
                return;
            }
        },
        None => None,
    };
    let replay = match state.message_send.turn_replay(task_id, turn_number).await {
        Ok(replay) => replay,
        Err(error) => {
            tracing::warn!(task_id, turn_number, %error, "Agent fallback could not read the turn");
            return;
        }
    };
    let failed_profile = match event.agent_profile.as_deref() {
        Some(profile) => AgentProfile::parse(profile),
        None => Some(task.agent_profile),
    };
    let facts = FallbackFacts {
        reason,
        failed_profile,
        loop_enabled,
        turn_was_fallback: replay.as_ref().map(|replay| replay.was_fallback),
    };
    let targets = match decide(&facts, &state.order) {
        Fallback::To(targets) => targets,
        Fallback::Skip(why) => {
            tracing::info!(task_id, turn_number, why, "Agent fallback skipped");
            return;
        }
    };
    let (Some(replay), Some(failed_profile)) = (replay, failed_profile) else {
        return;
    };
    let failed_harness = failed_profile.harness();

    let Some(run_as) = task
        .run_as_user_id
        .clone()
        .or_else(|| task.owner_user_id.clone())
        .or_else(|| task.created_by.clone())
    else {
        tracing::info!(
            task_id,
            "Agent fallback skipped: no user to run the task as"
        );
        return;
    };
    let run_as = ScopeContext::user(run_as);
    let resets_at = event.payload.get("resetsAt").and_then(|v| v.as_str());
    let sandbox_id = task.sandbox_id.as_deref().unwrap_or(task_id);
    let mut switched = None;
    for &harness in targets {
        let profile = match state
            .task_switch
            .profile_for(Some(sandbox_id), harness)
            .await
        {
            Ok(profile) => profile,
            Err(error) => {
                tracing::warn!(task_id, %harness, %error, "Agent fallback found no profile");
                continue;
            }
        };
        let summary = format!(
            "Switched from {} to {}: {}",
            label(failed_harness),
            label(harness),
            why(failed_harness, reason, resets_at),
        );
        let note = SwitchNote {
            summary: &summary,
            turn_number: Some(turn_number),
        };
        // Switched before queueing, rather than after: a sandbox fetching the
        // queued turn in between would run it on the refusing agent.
        let model = fallback_model(harness);
        match switch(state, task_id, profile, model, &run_as, &note).await {
            Ok(_) => {
                switched = Some((harness, profile));
                break;
            }
            Err(error) => {
                tracing::warn!(task_id, %profile, %error, "Agent fallback could not switch the agent");
            }
        }
    }
    let Some((harness, profile)) = switched else {
        return;
    };
    match state.message_send.queue_replay(replay).await {
        Ok(true) => {
            tracing::info!(
                task_id,
                turn_number,
                from = %failed_profile,
                to = %profile,
                "Agent fallback queued"
            );
            if let Err(error) = state.sandbox_waker.wake(task_id).await {
                state
                    .message_send
                    .persist_error_result(task_id, &error)
                    .await;
            }
        }
        // A redelivered event: the first delivery queued and woke.
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(task_id, %error, "Agent fallback could not queue the turn; switching back");
            let summary = format!(
                "Switched back to {}: the turn could not be queued for {}",
                label(failed_harness),
                label(harness),
            );
            let note = SwitchNote {
                summary: &summary,
                turn_number: Some(turn_number),
            };
            if let Err(error) = switch(
                state,
                task_id,
                failed_profile,
                task.model.as_deref(),
                &run_as,
                &note,
            )
            .await
            {
                tracing::warn!(task_id, %failed_profile, %error, "Agent fallback could not switch back");
            }
        }
    }
}

/// How the conversation names a harness.
fn label(harness: Harness) -> &'static str {
    match harness {
        Harness::Codex => "Codex",
        Harness::Pi => "Pi",
        Harness::MiniSwe => "mini-swe-agent",
        Harness::Acp => "Antigravity",
        Harness::ClaudeCode => "Claude Code",
    }
}

/// Why `harness` would not serve the turn, for one of [`FALLBACK_REASONS`].
fn why(harness: Harness, reason: Option<&str>, resets_at: Option<&str>) -> String {
    let name = label(harness);
    match (reason, resets_at) {
        (Some("usage_limit"), Some(at)) => format!("{name} hit its usage limit (resets at {at})"),
        (Some("usage_limit"), None) => format!("{name} hit its usage limit"),
        (Some("rate_limit"), _) => format!("{name} stayed rate-limited after retries"),
        (Some("provider_unavailable"), _) => {
            format!("{name}'s provider had no account available after retries")
        }
        (Some("connectivity"), _) => {
            format!("{name} could not reach its model provider after retries")
        }
        _ => format!("{name} would not serve the turn"),
    }
}

async fn switch(
    state: &FallbackPorts,
    task_id: &str,
    profile: AgentProfile,
    model: Option<&str>,
    run_as: &ScopeContext,
    note: &SwitchNote<'_>,
) -> Result<SwitchOutcome, TaskError> {
    let credentials = SwitchCredentials {
        hub_available: state.hub_available,
        secrets: state.secrets.as_ref(),
        provider_tokens: state.harness_credentials.as_ref(),
        credential_sync: &state.credential_sync,
    };
    state
        .task_switch
        .switch_for_fallback(task_id, profile, model, run_as, credentials, note)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(failed_profile: AgentProfile) -> FallbackFacts<'static> {
        FallbackFacts {
            reason: Some("usage_limit"),
            failed_profile: Some(failed_profile),
            loop_enabled: Some(true),
            turn_was_fallback: Some(false),
        }
    }

    fn decide_default(facts: &FallbackFacts<'_>) -> Vec<Harness> {
        match decide(facts, &AgentFallbackOrder::default()) {
            Fallback::To(targets) => targets.to_vec(),
            Fallback::Skip(_) => vec![],
        }
    }

    #[test]
    fn a_refused_turn_moves_to_the_other_agent() {
        assert_eq!(
            decide_default(&facts(AgentProfile::Codex)),
            [Harness::ClaudeCode]
        );
        assert_eq!(
            decide_default(&facts(AgentProfile::ClaudeCode)),
            [Harness::Codex]
        );
        assert_eq!(
            decide_default(&facts(AgentProfile::ClaudeCodeTui)),
            [Harness::Codex]
        );
        assert_eq!(
            decide_default(&facts(AgentProfile::Antigravity)),
            [Harness::ClaudeCode, Harness::Codex]
        );
    }

    /// Each reason arrives only once the sandbox's retries are spent.
    #[test]
    fn a_provider_that_will_not_serve_the_turn_falls_back() {
        for reason in [
            "usage_limit",
            "rate_limit",
            "provider_unavailable",
            "connectivity",
        ] {
            let facts = FallbackFacts {
                reason: Some(reason),
                ..facts(AgentProfile::Antigravity)
            };
            assert_eq!(
                decide_default(&facts),
                [Harness::ClaudeCode, Harness::Codex],
                "{reason}"
            );
        }
    }

    /// `invalid_input` covers a safety refusal, which must not reach the other
    /// provider.
    #[test]
    fn any_other_failure_stays_on_its_agent() {
        for reason in [
            None,
            Some("process_exit"),
            Some("unresponsive_process"),
            Some("internal"),
            Some("invalid_configuration"),
            Some("invalid_input"),
            Some("authentication"),
        ] {
            let facts = FallbackFacts {
                reason,
                ..facts(AgentProfile::Codex)
            };
            assert!(
                matches!(
                    decide(&facts, &AgentFallbackOrder::default()),
                    Fallback::Skip(_)
                ),
                "{reason:?}"
            );
        }
    }

    /// A Task the review gate or a person created has no Loop to ask.
    #[test]
    fn a_fallback_onto_claude_code_runs_opus() {
        assert_eq!(fallback_model(Harness::ClaudeCode), Some("opus"));
        assert_eq!(fallback_model(Harness::Codex), None);
        assert_eq!(
            ClaudeModel::from_alias("opus").map(ClaudeModel::api_id),
            Some("claude-opus-5-5")
        );
        assert_eq!(
            moatless_core::agent_catalog::validate_agent_model(
                AgentProfile::ClaudeCode,
                Some("opus")
            ),
            Ok(())
        );
    }

    /// A Task the review gate or a person created has no Loop to ask.
    #[test]
    fn a_task_no_loop_created_falls_back() {
        let facts = FallbackFacts {
            reason: Some("connectivity"),
            loop_enabled: None,
            ..facts(AgentProfile::Antigravity)
        };
        assert_eq!(
            decide_default(&facts),
            [Harness::ClaudeCode, Harness::Codex]
        );
    }

    #[test]
    fn a_loop_that_turned_fallback_off_stays_on_its_agent() {
        let facts = FallbackFacts {
            loop_enabled: Some(false),
            ..facts(AgentProfile::Codex)
        };
        assert_eq!(
            decide(&facts, &AgentFallbackOrder::default()),
            Fallback::Skip("the Loop turned fallback off")
        );
    }

    #[test]
    fn the_configured_order_decides_where_a_turn_goes() {
        let order = AgentFallbackOrder::parse("acp=codex;codex=").unwrap();
        assert_eq!(
            decide(&facts(AgentProfile::Antigravity), &order),
            Fallback::To(&[Harness::Codex])
        );
        assert!(matches!(
            decide(&facts(AgentProfile::Codex), &order),
            Fallback::Skip(_)
        ));
        assert!(matches!(
            decide(&facts(AgentProfile::Pi), &AgentFallbackOrder::default()),
            Fallback::Skip(_)
        ));
    }

    /// Every agent failed: the fallback turn fails too, and stops there, so
    /// its error is the Task's last word.
    #[test]
    fn a_fallback_turn_never_falls_back_again() {
        let facts = FallbackFacts {
            turn_was_fallback: Some(true),
            ..facts(AgentProfile::ClaudeCode)
        };
        assert_eq!(
            decide(&facts, &AgentFallbackOrder::default()),
            Fallback::Skip("the turn was already a fallback")
        );
    }

    #[test]
    fn the_switch_names_the_reason_the_provider_gave() {
        assert_eq!(
            why(
                Harness::Codex,
                Some("usage_limit"),
                Some("2026-10-03T18:05:00Z")
            ),
            "Codex hit its usage limit (resets at 2026-10-03T18:05:00Z)"
        );
        assert_eq!(
            why(Harness::ClaudeCode, Some("rate_limit"), None),
            "Claude Code stayed rate-limited after retries"
        );
        assert_eq!(
            why(Harness::Codex, Some("provider_unavailable"), None),
            "Codex's provider had no account available after retries"
        );
        assert_eq!(
            why(Harness::Acp, Some("connectivity"), None),
            "Antigravity could not reach its model provider after retries"
        );
    }

    #[test]
    fn a_profile_the_catalog_lacks_stays_where_it_is() {
        let facts = FallbackFacts {
            failed_profile: None,
            ..facts(AgentProfile::Codex)
        };
        assert_eq!(
            decide(&facts, &AgentFallbackOrder::default()),
            Fallback::Skip("the failed profile is not in the catalog")
        );
    }

    #[test]
    fn a_turn_with_no_input_has_nothing_to_re_run() {
        let facts = FallbackFacts {
            turn_was_fallback: None,
            ..facts(AgentProfile::Codex)
        };
        assert!(matches!(
            decide(&facts, &AgentFallbackOrder::default()),
            Fallback::Skip(_)
        ));
    }
}
