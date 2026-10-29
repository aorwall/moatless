//! [`TaskRooms`] against a real `task_bindings` table, with the hosting
//! Adapter faked.

use std::sync::Mutex;

use chrono::Utc;
use sqlx::PgPool;

use moatless_adapter_model::binding::{ADAPTER_EVENT_BINDING_ORIGIN, BindingDestination};
use moatless_adapter_model::capabilities::{Adapter, AdapterTaskRoom, RoomOpened};
use moatless_ports::identity::MockProviderIdentityLink;
use moatless_types::access_control::Scope;
use moatless_types::adapter::AdapterKind;

use super::*;

#[derive(Default)]
struct FakeRooms(Mutex<Vec<RoomRequest>>);

impl Adapter for FakeRooms {
    fn adapter_kind(&self) -> AdapterKind {
        AdapterKind::Slack
    }
}

#[async_trait::async_trait]
impl AdapterTaskRoom for FakeRooms {
    async fn open_room(
        &self,
        _connection: &AdapterConnection,
        request: &RoomRequest,
    ) -> Result<RoomOpened, moatless_adapter_model::error::AdapterError> {
        self.0.lock().unwrap().push(request.clone());
        Ok(RoomOpened {
            destination: BindingDestination::new(
                "slack_channel",
                "T1:C0ROOM".into(),
                serde_json::json!({ "channel": "C0ROOM", "team_id": "T1" }),
            ),
        })
    }
}

fn connection() -> AdapterConnection {
    AdapterConnection {
        id: "pc_slack".into(),
        adapter_kind: AdapterKind::Slack,
        connection_kind: "workspace".into(),
        scope_value: "T1".into(),
        credential_ref: None,
        webhook_secret: None,
        transport_config: serde_json::json!({}),
        created_by: "alice".into(),
        scope: Scope::Global,
        owner_user_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

const REQUESTER: &str = "00000000-0000-0000-0000-00000000000a";

struct Harness {
    rooms: TaskRooms,
    host: Arc<FakeRooms>,
    bindings: Arc<Bindings>,
}

async fn harness(pool: PgPool) -> Harness {
    let connections = moatless_adapter_model::compose::adapter_connections(pool.clone()).routing();
    connections.install(&connection()).await.unwrap();
    let host = Arc::new(FakeRooms::default());
    let mut adapters = Adapters::new();
    adapters.register_task_room(host.clone());
    let mut identities = MockProviderIdentityLink::new();
    identities
        .expect_provider_id_for_user()
        .returning(|_, user| Ok((user.to_string() == REQUESTER).then(|| "U0PERSON".into())));
    let bindings = crate::compose::loop_graph(pool).bindings();
    Harness {
        rooms: TaskRooms::new(
            Arc::new(adapters),
            connections,
            bindings.clone(),
            Arc::new(identities),
            "https://moatless.example/",
        ),
        host,
        bindings,
    }
}

fn delivery(anchor: Option<&str>) -> AdapterDelivery {
    let payload = RoomOpenPayload {
        task_id: "task-1".into(),
        title: "SP-1234".into(),
        room: RoomSpec {
            anchor: anchor.map(Into::into),
            ..super::tests::room()
        },
        requester: Some(REQUESTER.into()),
        run_as: None,
    };
    AdapterDelivery {
        id: "d1".into(),
        adapter: "slack".into(),
        kind: ROOM_OPEN_KIND.into(),
        event_id: "task-1".into(),
        payload: serde_json::to_value(payload).unwrap(),
        attempts: 1,
        max_attempts: 5,
    }
}

async fn bind_thread(bindings: &Bindings) {
    bindings
        .bind(
            &connection(),
            None,
            "task-1",
            "slack",
            "slack_thread",
            "T1:C0ASK:1700000000.000100",
            serde_json::json!({ "channel": "C0ASK", "thread_ts": "1700000000.000100" }),
            ADAPTER_EVENT_BINDING_ORIGIN,
            None,
            &BindScope::default(),
        )
        .await
        .unwrap();
}

#[sqlx::test]
async fn a_room_opens_from_the_tasks_own_thread_and_binds_once(pool: PgPool) {
    let h = harness(pool).await;
    bind_thread(&h.bindings).await;

    h.rooms.execute(&delivery(None)).await.unwrap();
    h.rooms.execute(&delivery(None)).await.unwrap();

    let calls = h.host.0.lock().unwrap().clone();
    assert_eq!(calls.len(), 1, "a bound room is not opened again");
    let request = &calls[0];
    assert_eq!(
        request.origin.as_ref().map(|b| b.subject_kind.as_str()),
        Some("slack_thread")
    );
    assert_eq!(request.invitees, vec!["U0PERSON".to_string()]);
    assert_eq!(request.private, Some(true));
    assert_eq!(
        request.task_url.as_deref(),
        Some("https://moatless.example/tasks/task-1")
    );

    let room = h
        .bindings
        .for_task("task-1")
        .await
        .unwrap()
        .into_iter()
        .find(|b| b.origin == ROOM_BINDING_ORIGIN)
        .expect("the room is bound");
    assert_eq!(room.subject_kind, "slack_channel");
    assert_eq!(room.subject_key, "T1:C0ROOM");
}

#[sqlx::test]
async fn a_room_with_an_anchor_opens_before_any_binding(pool: PgPool) {
    let h = harness(pool).await;

    h.rooms
        .execute(&delivery(Some("channel:C0FACTORY")))
        .await
        .unwrap();

    let calls = h.host.0.lock().unwrap().clone();
    assert!(calls[0].origin.is_none());
    assert_eq!(calls[0].anchor.as_deref(), Some("channel:C0FACTORY"));
}

/// The ingress path binds the triggering thread just after the Task opens.
#[sqlx::test]
async fn a_room_with_nothing_to_open_from_yet_is_retried(pool: PgPool) {
    let h = harness(pool).await;

    let failure = h.rooms.execute(&delivery(None)).await.unwrap_err();

    assert!(!failure.is_permanent());
    assert!(h.host.0.lock().unwrap().is_empty());
}

#[sqlx::test]
async fn a_thread_is_held_by_the_one_task_actively_bound_to_it(pool: PgPool) {
    let h = harness(pool).await;
    bind_thread(&h.bindings).await;
    let held_by_another = |task_id: &'static str| {
        let bindings = h.bindings.clone();
        async move {
            bindings
                .held_by_another(
                    "slack",
                    "slack_thread",
                    "T1:C0ASK:1700000000.000100",
                    task_id,
                )
                .await
                .unwrap()
                .map(|b| b.task_id)
        }
    };

    assert_eq!(held_by_another("task-2").await.as_deref(), Some("task-1"));
    assert_eq!(held_by_another("task-1").await, None, "its own thread");

    h.bindings.deactivate_for_task("task-1").await.unwrap();
    assert_eq!(
        held_by_another("task-2").await,
        None,
        "a released thread is free"
    );
}
