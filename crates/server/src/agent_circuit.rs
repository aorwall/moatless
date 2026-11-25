//! Routing a Loop Task's next turn around a harness that is cooling down.
//!
//! [`fall_back_to_other_agent`](crate::event_handler::agent_fallback) records
//! a cooldown when a harness refuses a turn. Every Loop delivery passes
//! through [`LoopTurnAcceptor`], which moves the Task onto the fallback while
//! the Loop's agent cools down and back once it no longer does, so a firing
//! never spends a turn on an agent already known to refuse it.

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use moatless_ports::task::{InboundTurn, MessageAcceptor};
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task::switch::SwitchNote;
use moatless_types::access_control::ScopeContext;
use moatless_types::agent::Harness;
use moatless_types::store::StoreError;

use crate::event_handler::agent_fallback::{
    FallbackPorts, cooling_until, fallback_model, label, run_as_user, serving, switch,
};

/// Where a Loop Task's next turn runs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Route {
    Stay,
    /// From `from` onto `harness`, while `from` cools down until `until`.
    Around {
        from: Harness,
        harness: Harness,
        until: DateTime<Utc>,
    },
    /// Back onto the Loop's own agent, whose cooldown has ended.
    Back,
}

/// What decides a [`Route`], read from the Task, its Loop and the cooldowns.
#[derive(Debug)]
pub(crate) struct RouteFacts<'a> {
    pub loop_harness: Harness,
    pub task_harness: Harness,
    /// Until when the Loop's harness cools down for the run-as user.
    pub loop_cooling_until: Option<DateTime<Utc>>,
    /// `AGENT_FALLBACK_ORDER`'s targets for the Loop's harness, in order.
    pub targets: &'a [Harness],
    /// Until when the Task's own harness cools down, when it is neither the
    /// Loop's nor one of `targets`, such as a review's per-head agent.
    pub task_cooling_until: Option<DateTime<Utc>>,
    /// `AGENT_FALLBACK_ORDER`'s targets for the Task's harness, in order.
    pub task_targets: &'a [Harness],
    /// Which of `targets` and `task_targets` cool down for the run-as user.
    pub cooling: &'a [Harness],
}

/// A Task on a harness that is neither the Loop's nor one of its fallbacks
/// stays there while that harness serves, rather than undoing a person's
/// switch or a per-head agent; while it cools down, the Task moves onto its own
/// fallback. A person's switch onto a fallback is undone once the Loop's agent
/// serves: no row tells it apart from the platform's.
pub(crate) fn route(facts: &RouteFacts<'_>) -> Route {
    let elsewhere =
        facts.task_harness != facts.loop_harness && !facts.targets.contains(&facts.task_harness);
    if elsewhere {
        return match facts.task_cooling_until {
            Some(until) => around(facts.task_harness, facts.task_targets, facts.cooling, until),
            None => Route::Stay,
        };
    }
    match facts.loop_cooling_until {
        Some(until) if facts.task_harness == facts.loop_harness => {
            around(facts.loop_harness, facts.targets, facts.cooling, until)
        }
        Some(_) => Route::Stay,
        None if facts.task_harness != facts.loop_harness => Route::Back,
        None => Route::Stay,
    }
}

/// Onto the first of `targets` that serves. None serving stays: `from` may
/// serve sooner than a fallback.
fn around(from: Harness, targets: &[Harness], cooling: &[Harness], until: DateTime<Utc>) -> Route {
    match serving(targets, cooling).first() {
        Some(&harness) => Route::Around {
            from,
            harness,
            until,
        },
        None => Route::Stay,
    }
}

/// The [`MessageAcceptor`] every Loop delivery uses: routes the Task by
/// [`route`], then accepts the turn. A failed read or switch is logged and the
/// turn is still accepted, on whichever agent the Task holds.
pub struct LoopTurnAcceptor {
    ports: FallbackPorts,
}

impl LoopTurnAcceptor {
    pub fn new(ports: FallbackPorts) -> Self {
        Self { ports }
    }

    async fn route_task(&self, task_id: &str) {
        // reason: a Loop delivery runs as the platform; `run_as` scopes the
        // credential checks of any switch.
        #[allow(clippy::disallowed_methods)]
        let system = ScopeContext::system();
        let task = match self.ports.tasks.find_by_id(task_id, &system).await {
            Ok(Some(task)) if task.status != TaskStatus::Closed => task,
            Ok(_) => return,
            Err(error) => {
                tracing::warn!(task_id, %error, "Agent circuit could not read the task");
                return;
            }
        };
        let Some(loop_id) = task.loop_id.as_deref() else {
            return;
        };
        let choice = match self.ports.loops.agent_choice(loop_id).await {
            Ok(Some(choice)) if choice.fallback_to_other_agent => choice,
            Ok(_) => return,
            Err(error) => {
                tracing::warn!(task_id, loop_id, %error, "Agent circuit could not read the loop");
                return;
            }
        };
        let Some(user) = run_as_user(&task) else {
            return;
        };
        let loop_harness = choice.agent_profile.harness();
        let task_harness = task.agent_profile.harness();
        let targets = self.ports.order.targets(loop_harness);
        let task_targets = self.ports.order.targets(task_harness);
        if targets.is_empty() && task_targets.is_empty() {
            return;
        }
        let mut cooling = Vec::new();
        for &target in targets.iter().chain(task_targets) {
            if !cooling.contains(&target)
                && cooling_until(&self.ports, &user, target).await.is_some()
            {
                cooling.push(target);
            }
        }
        let facts = RouteFacts {
            loop_harness,
            task_harness,
            loop_cooling_until: cooling_until(&self.ports, &user, loop_harness).await,
            targets,
            task_cooling_until: cooling_until(&self.ports, &user, task_harness).await,
            task_targets,
            cooling: &cooling,
        };
        let (profile, model, summary) = match route(&facts) {
            Route::Stay => return,
            Route::Around {
                from,
                harness,
                until,
            } => {
                let sandbox_id = task.sandbox_id.as_deref().unwrap_or(task_id);
                let profile = match self
                    .ports
                    .task_switch
                    .profile_for(Some(sandbox_id), harness)
                    .await
                {
                    Ok(profile) => profile,
                    Err(error) => {
                        tracing::warn!(task_id, %error, "Agent circuit found no profile");
                        return;
                    }
                };
                let summary = format!(
                    "Switched from {} to {}: {} is cooling down until {}",
                    label(from),
                    label(harness),
                    label(from),
                    until.to_rfc3339_opts(SecondsFormat::Secs, true),
                );
                (
                    profile,
                    fallback_model(harness).map(str::to_string),
                    summary,
                )
            }
            Route::Back => (
                choice.agent_profile,
                choice.model.clone(),
                format!(
                    "Switched back to {}: its cooldown ended",
                    label(loop_harness)
                ),
            ),
        };
        let note = SwitchNote {
            summary: &summary,
            turn_number: None,
        };
        let run_as = ScopeContext::user(user);
        match switch(
            &self.ports,
            task_id,
            profile,
            model.as_deref(),
            &run_as,
            &note,
        )
        .await
        {
            Ok(_) => tracing::info!(task_id, %profile, "Agent circuit switched the task"),
            Err(error) => {
                tracing::warn!(task_id, %profile, %error, "Agent circuit could not switch the task");
            }
        }
    }
}

#[async_trait]
impl MessageAcceptor for LoopTurnAcceptor {
    async fn accept(&self, turn: InboundTurn) -> Result<(), StoreError> {
        self.route_task(&turn.task_id).await;
        self.ports.message_send.accept(turn).await
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    const ACP_TARGETS: &[Harness] = &[Harness::ClaudeCode, Harness::Codex];

    fn facts(task_harness: Harness, loop_cooling: bool, cooling: &[Harness]) -> RouteFacts<'_> {
        RouteFacts {
            loop_harness: Harness::Acp,
            task_harness,
            loop_cooling_until: loop_cooling.then(|| Utc::now() + Duration::hours(1)),
            targets: ACP_TARGETS,
            task_cooling_until: None,
            task_targets: &[],
            cooling,
        }
    }

    #[test]
    fn a_turn_for_a_cooling_agent_goes_to_its_first_serving_fallback() {
        let first = facts(Harness::Acp, true, &[]);
        let until = first.loop_cooling_until.expect("cooling");
        assert_eq!(
            route(&first),
            Route::Around {
                from: Harness::Acp,
                harness: Harness::ClaudeCode,
                until
            }
        );
        assert_eq!(fallback_model(Harness::ClaudeCode), Some("opus"));

        let claude_cooling = [Harness::ClaudeCode];
        let second = RouteFacts {
            loop_cooling_until: Some(until),
            ..facts(Harness::Acp, true, &claude_cooling)
        };
        assert_eq!(
            route(&second),
            Route::Around {
                from: Harness::Acp,
                harness: Harness::Codex,
                until
            }
        );
    }

    #[test]
    fn every_agent_cooling_leaves_the_task_where_it_is() {
        for task_harness in [Harness::Acp, Harness::ClaudeCode] {
            let facts = facts(task_harness, true, ACP_TARGETS);
            assert_eq!(route(&facts), Route::Stay, "{task_harness}");
        }
    }

    #[test]
    fn a_task_on_a_fallback_returns_once_the_cooldown_ends() {
        for task_harness in ACP_TARGETS {
            assert_eq!(route(&facts(*task_harness, false, &[])), Route::Back);
            assert_eq!(route(&facts(*task_harness, true, &[])), Route::Stay);
        }
    }

    #[test]
    fn a_task_on_its_loops_agent_stays_while_it_serves() {
        assert_eq!(route(&facts(Harness::Acp, false, &[])), Route::Stay);
    }

    /// The review gate's case: a Claude Code Loop whose Task starts on
    /// Antigravity, picked per head, while Antigravity cools down.
    #[test]
    fn a_task_started_on_another_cooling_agent_goes_to_that_agents_fallback() {
        let until = Utc::now() + Duration::hours(1);
        let facts = RouteFacts {
            loop_harness: Harness::ClaudeCode,
            task_harness: Harness::Acp,
            loop_cooling_until: None,
            targets: &[Harness::Codex],
            task_cooling_until: Some(until),
            task_targets: ACP_TARGETS,
            cooling: &[],
        };
        assert_eq!(
            route(&facts),
            Route::Around {
                from: Harness::Acp,
                harness: Harness::ClaudeCode,
                until
            }
        );
        let serving = RouteFacts {
            task_cooling_until: None,
            ..facts
        };
        assert_eq!(route(&serving), Route::Stay);
    }

    #[test]
    fn a_task_a_person_moved_elsewhere_is_left_alone() {
        for cooling in [true, false] {
            assert_eq!(route(&facts(Harness::Pi, cooling, &[])), Route::Stay);
        }
    }
}
