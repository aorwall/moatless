//! Unit tests for one reconciler tick: which candidates it enumerates, what it
//! confirms against an authoritative read, and how the attempt budget bounds a
//! retry.

use super::*;

use moatless_types::task::CheckoutMode;

use super::super::dao::IntentWrite;
use super::super::events::LifecycleEvent;
use super::super::orchestrator::{OutstandingCommand, StoreError, TaskIntent};
use moatless_sandbox_runtime::operation::{IntentEffect, OperationGeneration};

struct MemberStore {
    members: Result<Vec<SandboxMember>, StoreError>,
}

#[async_trait]
impl LifecycleDao for MemberStore {
    async fn load(&self, _task_id: &str) -> Result<Option<TaskIntent>, StoreError> {
        panic!("member registration test must not load task intent")
    }

    async fn reconcile_candidates(
        &self,
    ) -> Result<Vec<crate::lifecycle::dao::ReconcileCandidate>, StoreError> {
        panic!("member registration test must not enumerate reconcile candidates")
    }

    async fn running_sandbox_members(&self) -> Result<Vec<SandboxMember>, StoreError> {
        match &self.members {
            Ok(members) => Ok(members.clone()),
            Err(e) => Err(StoreError::new(e.to_string())),
        }
    }

    async fn sandbox_shape(
        &self,
        _sandbox_id: &str,
    ) -> Result<Option<crate::lifecycle::dao::SandboxShape>, StoreError> {
        panic!("member registration test must not read a sandbox shape")
    }

    async fn has_other_active_members(
        &self,
        _sandbox_id: &str,
        _task_id: &str,
    ) -> Result<bool, StoreError> {
        panic!("member registration test must not enumerate active peers")
    }

    async fn allocate_generation(&self, _task_id: &str) -> Result<OperationGeneration, StoreError> {
        panic!("member registration test must not allocate generations")
    }

    async fn persist_intent(
        &self,
        _task_id: &str,
        _write: &IntentWrite,
    ) -> Result<IntentEffect, StoreError> {
        panic!("member registration test must not persist intent")
    }

    async fn commit_fact_and_clear_marker(
        &self,
        _sandbox_id: &str,
        _task_id: &str,
        _event: &LifecycleEvent,
    ) -> Result<(), StoreError> {
        panic!("member registration test must not commit lifecycle facts")
    }

    async fn clear_marker(
        &self,
        _task_id: &str,
        _generation: OperationGeneration,
    ) -> Result<(), StoreError> {
        panic!("member registration test must not clear markers")
    }

    async fn stopped_with_storage(
        &self,
        _task_ids: &[String],
    ) -> Result<Vec<(String, DateTime<Utc>)>, StoreError> {
        panic!("member registration test must not enumerate stopped storage")
    }

    async fn mark_storage_reclaimed(&self, _task_id: &str) -> Result<(), StoreError> {
        panic!("member registration test must not mark storage reclaimed")
    }
    async fn record_running_commands(
        &self,
        _task_id: &str,
        _running: i32,
    ) -> Result<(), StoreError> {
        panic!("member registration test must not record running commands")
    }
    async fn record_unreported_commands(
        &self,
        _task_id: &str,
        _commands: &[OutstandingCommand],
        _revision: i64,
    ) -> Result<(), StoreError> {
        panic!("member registration test must not record unreported commands")
    }
    async fn take_unreported_commands(
        &self,
        _task_id: &str,
    ) -> Result<Vec<OutstandingCommand>, StoreError> {
        panic!("member registration test must not take the unreported commands")
    }
}

#[derive(Default)]
struct RecordingRegistry {
    calls: Mutex<Vec<SandboxMember>>,
}

#[async_trait]
impl MemberRegistryPort for RecordingRegistry {
    async fn ensure_registered(&self, member: &SandboxMember) -> Result<(), MemberRegistryError> {
        self.calls.lock().unwrap().push(member.clone());
        Ok(())
    }
}

/// Fails every registration with a fixed `MemberRegistryError` carrying a
/// `source`, exercising the failure/dedup path and the source-chain log.
struct FailingRegistry {
    context: String,
    source: String,
}

#[async_trait]
impl MemberRegistryPort for FailingRegistry {
    async fn ensure_registered(&self, _member: &SandboxMember) -> Result<(), MemberRegistryError> {
        Err(MemberRegistryError::with_source(
            self.context.clone(),
            std::io::Error::other(self.source.clone()),
        ))
    }
}

fn member(task_id: &str, sandbox_id: &str) -> SandboxMember {
    SandboxMember {
        task_id: task_id.to_string(),
        sandbox_id: sandbox_id.to_string(),
        checkout_mode: CheckoutMode::Shared,
        agent_profile: AgentProfile::ClaudeCode,
    }
}

fn empty_failures() -> Mutex<HashMap<MemberKey, MemberFailure>> {
    Mutex::new(HashMap::new())
}

fn t0() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

#[tokio::test]
async fn running_members_are_registered_from_durable_store() {
    let members = vec![member("lead", "sandbox-1"), member("guest", "sandbox-1")];
    let store = MemberStore {
        members: Ok(members.clone()),
    };
    let registry = RecordingRegistry::default();

    ensure_running_members_registered(
        &store,
        &registry,
        &empty_failures(),
        t0(),
        MEMBER_REG_RELOG_INTERVAL,
    )
    .await;

    assert_eq!(*registry.calls.lock().unwrap(), members);
}

#[tokio::test]
async fn member_enumeration_failure_registers_nothing() {
    let store = MemberStore {
        members: Err(StoreError::new("database unavailable")),
    };
    let registry = RecordingRegistry::default();

    ensure_running_members_registered(
        &store,
        &registry,
        &empty_failures(),
        t0(),
        MEMBER_REG_RELOG_INTERVAL,
    )
    .await;

    assert!(registry.calls.lock().unwrap().is_empty());
}

#[test]
fn first_failure_warns_then_identical_repeat_suppresses_within_window() {
    let mut failures = HashMap::new();
    let key = ("sandbox-1".to_string(), "task-1".to_string());

    let first = note_member_failure(
        &mut failures,
        key.clone(),
        "JoinTask failed: unavailable",
        t0(),
        MEMBER_REG_RELOG_INTERVAL,
    );
    assert_eq!(first, MemberFailureLog::Warn { suppressed: 0 });

    let repeat = note_member_failure(
        &mut failures,
        key.clone(),
        "JoinTask failed: unavailable",
        t0() + chrono::Duration::seconds(30),
        MEMBER_REG_RELOG_INTERVAL,
    );
    assert_eq!(repeat, MemberFailureLog::Suppress);
    assert_eq!(failures.get(&key).unwrap().suppressed_since_warn, 1);
}

#[test]
fn changed_error_warns_immediately_and_reports_suppressed_count() {
    let mut failures = HashMap::new();
    let key = ("sandbox-1".to_string(), "task-1".to_string());

    note_member_failure(
        &mut failures,
        key.clone(),
        "err-a",
        t0(),
        MEMBER_REG_RELOG_INTERVAL,
    );
    // Two identical repeats are hidden.
    note_member_failure(
        &mut failures,
        key.clone(),
        "err-a",
        t0() + chrono::Duration::seconds(30),
        MEMBER_REG_RELOG_INTERVAL,
    );
    note_member_failure(
        &mut failures,
        key.clone(),
        "err-a",
        t0() + chrono::Duration::seconds(60),
        MEMBER_REG_RELOG_INTERVAL,
    );

    let changed = note_member_failure(
        &mut failures,
        key.clone(),
        "err-b",
        t0() + chrono::Duration::seconds(90),
        MEMBER_REG_RELOG_INTERVAL,
    );
    assert_eq!(changed, MemberFailureLog::Warn { suppressed: 2 });
    assert_eq!(failures.get(&key).unwrap().last_error, "err-b");
    assert_eq!(failures.get(&key).unwrap().suppressed_since_warn, 0);
}

#[test]
fn identical_failure_rewarns_after_relog_interval() {
    let mut failures = HashMap::new();
    let key = ("sandbox-1".to_string(), "task-1".to_string());

    note_member_failure(
        &mut failures,
        key.clone(),
        "err",
        t0(),
        MEMBER_REG_RELOG_INTERVAL,
    );
    let after = note_member_failure(
        &mut failures,
        key.clone(),
        "err",
        t0() + chrono::Duration::from_std(MEMBER_REG_RELOG_INTERVAL).unwrap(),
        MEMBER_REG_RELOG_INTERVAL,
    );
    assert!(matches!(after, MemberFailureLog::Warn { .. }));
}

#[tokio::test]
async fn repeated_registration_failure_accumulates_suppressed_count() {
    let members = vec![member("task-1", "sandbox-1")];
    let store = MemberStore {
        members: Ok(members),
    };
    let registry = FailingRegistry {
        context: "JoinTask failed".to_string(),
        source: "status: FailedPrecondition, message: clone failed".to_string(),
    };
    let failures = empty_failures();

    ensure_running_members_registered(
        &store,
        &registry,
        &failures,
        t0(),
        MEMBER_REG_RELOG_INTERVAL,
    )
    .await;
    ensure_running_members_registered(
        &store,
        &registry,
        &failures,
        t0() + chrono::Duration::seconds(30),
        MEMBER_REG_RELOG_INTERVAL,
    )
    .await;

    let key = ("sandbox-1".to_string(), "task-1".to_string());
    let guard = failures.lock().unwrap();
    let entry = guard.get(&key).expect("failure recorded");
    assert_eq!(entry.suppressed_since_warn, 1);
    // The source chain is captured, not just the bare context.
    assert!(
        entry.last_error.contains("clone failed"),
        "chain: {}",
        entry.last_error
    );
}

#[tokio::test]
async fn success_clears_failure_state_and_absent_members_are_pruned() {
    // Seed a stale failure for a member that will not appear this tick, plus
    // a member that registers successfully.
    let failures = empty_failures();
    failures.lock().unwrap().insert(
        ("gone-sandbox".to_string(), "gone-task".to_string()),
        MemberFailure {
            last_error: "stale".to_string(),
            last_warned_at: t0(),
            suppressed_since_warn: 3,
        },
    );
    failures.lock().unwrap().insert(
        ("sandbox-1".to_string(), "task-1".to_string()),
        MemberFailure {
            last_error: "was failing".to_string(),
            last_warned_at: t0(),
            suppressed_since_warn: 1,
        },
    );

    let store = MemberStore {
        members: Ok(vec![member("task-1", "sandbox-1")]),
    };
    let registry = RecordingRegistry::default();

    ensure_running_members_registered(
        &store,
        &registry,
        &failures,
        t0() + chrono::Duration::seconds(30),
        MEMBER_REG_RELOG_INTERVAL,
    )
    .await;

    // Successful member cleared, absent member pruned → map empty.
    assert!(failures.lock().unwrap().is_empty());
}

#[test]
fn format_error_chain_includes_source() {
    let err = MemberRegistryError::with_source(
        "JoinTask failed",
        std::io::Error::other("status: Unimplemented"),
    );
    assert_eq!(
        format_error_chain(&err),
        "JoinTask failed: status: Unimplemented"
    );
}
