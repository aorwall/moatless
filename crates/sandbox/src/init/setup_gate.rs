//! Holds every Member's agent turns until the pod's workspace setup has run.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;
use tracing::warn;

/// How long a turn waits on setup commands before it starts anyway, counted
/// from [`SetupGate::begin_setup`]. A setup command has no timeout of its own,
/// so without this a hung one would queue turns forever.
const SETUP_WAIT_CAP: Duration = Duration::from_mins(15);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Core init is restoring or cloning. Uncapped: a failed core init exits
    /// the pod (`app::run_init`), so nothing waits on a pod that stays up.
    CoreInit,
    /// Setup commands are running. Every waiter, including a Member that joins
    /// mid-setup, gives up at this one pod-wide deadline.
    SettingUp {
        deadline: Instant,
    },
    Open,
}

/// Opens once workspace setup has finished; the agent command loop waits on it.
///
/// Opened by [`crate::init::initialize`] beside the `.workspace-ready` sentinel.
#[derive(Clone)]
pub struct SetupGate(Arc<watch::Sender<Stage>>);

impl SetupGate {
    /// A gate that holds turns until [`SetupGate::open`].
    pub fn closed() -> Self {
        Self(Arc::new(watch::Sender::new(Stage::CoreInit)))
    }

    /// A gate that never holds a turn.
    pub fn opened() -> Self {
        Self(Arc::new(watch::Sender::new(Stage::Open)))
    }

    /// Marks the start of the setup commands, which starts [`SETUP_WAIT_CAP`].
    pub fn begin_setup(&self) {
        self.0.send_if_modified(|stage| {
            let starting = *stage == Stage::CoreInit;
            if starting {
                *stage = Stage::SettingUp {
                    deadline: Instant::now() + SETUP_WAIT_CAP,
                };
            }
            starting
        });
    }

    /// Releases every current and future waiter.
    pub fn open(&self) {
        self.0.send_replace(Stage::Open);
    }

    /// Returns once the gate is open, or at the deadline [`SetupGate::begin_setup`]
    /// set, which a waiter that arrives mid-setup shares.
    pub(crate) async fn wait(&self) {
        let mut stage = self.0.subscribe();
        // `self` holds the sender, so neither `wait_for` sees it dropped.
        let started = match stage.wait_for(|s| *s != Stage::CoreInit).await {
            Ok(started) => *started,
            Err(_) => return,
        };
        let Stage::SettingUp { deadline } = started else {
            return;
        };
        if tokio::time::timeout_at(deadline, stage.wait_for(|s| *s == Stage::Open))
            .await
            .is_err()
        {
            warn!(
                cap_secs = SETUP_WAIT_CAP.as_secs(),
                "Workspace setup still running; starting agent turns anyway"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn wait_gives_up_at_the_cap_counted_from_setup_start() {
        let gate = SetupGate::closed();
        let waiter = tokio::spawn({
            let gate = gate.clone();
            async move { gate.wait().await }
        });

        // A slow core init spends no part of the setup budget.
        tokio::time::sleep(SETUP_WAIT_CAP * 2).await;
        assert!(!waiter.is_finished());

        gate.begin_setup();
        let setup_started = tokio::time::Instant::now();
        waiter.await.unwrap();

        assert_eq!(setup_started.elapsed(), SETUP_WAIT_CAP);
    }

    #[tokio::test(start_paused = true)]
    async fn a_member_joining_mid_setup_shares_the_setup_deadline() {
        let gate = SetupGate::closed();
        gate.begin_setup();
        let setup_started = tokio::time::Instant::now();
        tokio::time::sleep(Duration::from_mins(14)).await;

        gate.wait().await;

        assert_eq!(setup_started.elapsed(), SETUP_WAIT_CAP);
    }
}
