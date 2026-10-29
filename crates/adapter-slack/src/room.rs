//! Slack's Task Room: a code channel, opened from the message that started
//! the Task and bound as one [`subject::room_subject_kind`] subject.

use moatless_adapter_model::binding::{BindingDestination, BindingProbe};
use moatless_adapter_model::capabilities::Adapter;
use moatless_adapter_model::capabilities::{AdapterTaskRoom, RoomOpened, RoomRequest};
use moatless_adapter_model::connections::ConnectionLookup;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::loops::delivery_only_subscription;
use moatless_types::adapter::{AdapterConnection, AdapterKind};
use tracing::warn;

use super::adapter::SlackAdapter;
use super::api::{CodeChannelRequest, SlackApi};
use super::ingress::Routed;
use super::subject;

/// `name` is a display name, spaces allowed; Slack truncates a longer one.
const MAX_NAME_CHARS: usize = 80;

#[async_trait::async_trait]
impl AdapterTaskRoom for SlackAdapter {
    async fn open_room(
        &self,
        connection: &AdapterConnection,
        request: &RoomRequest,
    ) -> Result<RoomOpened, AdapterError> {
        let app = self
            .app(moatless_adapter_model::apps::app_key_from_transport_config(
                &connection.transport_config,
            ))
            .await?;
        let (team, origin_channel, origin_ts) = match (&request.origin, &request.anchor) {
            (Some(binding), _) => {
                let thread = subject::thread_coordinates(&binding.destination_config);
                let (Some(channel), Some(ts)) = (thread.channel, thread.thread_ts) else {
                    return Err(SharedAdapterError::BadRequest(
                        "the Task's Slack Binding names no thread to open a room from".into(),
                    )
                    .into());
                };
                let team = binding.subject_key.split(':').next().unwrap_or("default");
                (team.to_string(), channel.to_string(), ts.to_string())
            }
            (None, Some(anchor)) => {
                let (channel, ts) = announce(&app.api, anchor, request).await?;
                (connection.scope_value.clone(), channel, ts)
            }
            (None, None) => {
                return Err(SharedAdapterError::BadRequest(
                    "a Slack Task Room needs a Slack origin or an `in` channel".into(),
                )
                .into());
            }
        };

        let name: String = request.title.chars().take(MAX_NAME_CHARS).collect();
        let channel = app
            .api
            .create_code_channel(&CodeChannelRequest {
                name: &name,
                session_id: &request.task_id,
                origin: Some((&origin_channel, &origin_ts)),
                private: request.private,
            })
            .await?;
        // Deliberately not fatal: the room exists and is bound either way, and
        // a missing scope would fail every retry the same way.
        if !request.invitees.is_empty() {
            if let Err(e) = app.api.invite(&channel, &request.invitees).await {
                warn!(task_id = %request.task_id, channel, error = %e, "Task Room invite failed");
            }
        }

        Ok(RoomOpened {
            destination: subject::room_destination(&team, &channel),
        })
    }
}

/// The route for a message in a Task Room's channel, or `None` when `channel`
/// holds no room.
///
/// Probed before the Loop lookup, so no Task is ever created from a room. A
/// message in a `thread` another Task holds goes to that Task, not the room's.
pub(super) async fn route(
    adapter_connections: &dyn ConnectionLookup,
    task_bindings: &dyn BindingProbe,
    adapter: &SlackAdapter,
    team_id: &str,
    channel: &str,
    thread: Option<&BindingDestination>,
) -> Result<Option<Routed>, AdapterError> {
    let room = subject::room_destination(team_id, channel);
    let is_room = task_bindings
        .is_bound(adapter.provider_key(), room.subject_kind, &room.subject_key)
        .await?;
    if !is_room {
        return Ok(None);
    }
    let Some(connection) = adapter_connections.by_scope_value("slack", team_id).await? else {
        return Ok(None);
    };
    let destination = match thread {
        Some(thread)
            if task_bindings
                .is_actively_bound(
                    adapter.provider_key(),
                    thread.subject_kind,
                    &thread.subject_key,
                )
                .await? =>
        {
            thread.clone()
        }
        _ => room,
    };
    let subscription = delivery_only_subscription(&connection, AdapterKind::Slack, channel);
    Ok(Some(Routed {
        config: moatless_adapter_model::loops::subscription_context(&subscription),
        source_id: channel.to_string(),
        app_key: moatless_adapter_model::apps::app_key_from_transport_config(
            &connection.transport_config,
        )
        .to_string(),
        connection_id: connection.id,
        subscription_id: subscription.id,
        destination,
        is_tracked_thread: true,
    }))
}

/// Post the message the room is opened from into `anchor`, returning its
/// `(channel, ts)`.
///
/// A retry after this succeeded but the channel did not posts it again.
async fn announce(
    api: &SlackApi,
    anchor: &str,
    request: &RoomRequest,
) -> Result<(String, String), AdapterError> {
    let (channel, thread_ts) = subject::parse_destination(anchor)?;
    let text = match &request.task_url {
        Some(url) => format!("*{}* · <{url}|open task>", request.title),
        None => format!("*{}*", request.title),
    };
    let ts = api
        .post_message(&channel, &text, thread_ts.as_deref())
        .await?
        .ok_or_else(|| SharedAdapterError::AdapterApi("chat.postMessage: missing ts".into()))?;
    Ok((channel, ts))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use moatless_adapter_model::capabilities::{AdapterTaskRoom, RoomRequest};
    use moatless_adapter_model::test_support::ProviderStub;
    use moatless_types::access_control::Scope;
    use moatless_types::adapter::{AdapterConnection, AdapterKind, TaskBinding};

    use crate::adapter::SlackAdapter;
    use crate::api::SlackApi;

    async fn slack() -> ProviderStub {
        ProviderStub::start(|path| match path {
            "/agents.conversations.create" => {
                serde_json::json!({ "ok": true, "channel_id": "C0ROOM" })
            }
            "/chat.postMessage" => serde_json::json!({ "ok": true, "ts": "1700000000.000900" }),
            _ => serde_json::json!({ "ok": true }),
        })
        .await
    }

    fn adapter(stub: &ProviderStub) -> SlackAdapter {
        SlackAdapter::with_api(SlackApi::with_base_url("xoxb", stub.base_url()), "s", None)
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

    fn request(origin: Option<TaskBinding>, anchor: Option<&str>) -> RoomRequest {
        RoomRequest {
            task_id: "task-1".into(),
            title: "SP-1234".into(),
            origin,
            anchor: anchor.map(Into::into),
            invitees: vec!["U0PERSON".into()],
            private: Some(true),
            task_url: Some("https://moatless.example/tasks/task-1".into()),
        }
    }

    fn thread_binding() -> TaskBinding {
        let now = Utc::now();
        TaskBinding {
            id: "tb_1".into(),
            task_id: "task-1".into(),
            adapter_kind: AdapterKind::Slack,
            subject_kind: "slack_thread".into(),
            subject_key: "T1:C0ASK:1700000000.000100".into(),
            connection_id: "pc_slack".into(),
            loop_id: None,
            destination_config: serde_json::json!({
                "channel": "C0ASK",
                "thread_ts": "1700000000.000100",
            }),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: "ingress".into(),
            created_at: now,
            last_event_at: now,
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: None,
            active: true,
            deactivated_at: None,
            watching: true,
        }
    }

    fn body(stub: &ProviderStub, path: &str) -> serde_json::Value {
        let request = stub
            .requests()
            .into_iter()
            .find(|r| r.path == path)
            .unwrap_or_else(|| panic!("no {path} call"));
        serde_json::from_str(&request.body).unwrap()
    }

    #[tokio::test]
    async fn a_room_opens_from_the_triggering_thread_keyed_by_the_task() {
        let stub = slack().await;

        let opened = adapter(&stub)
            .open_room(&connection(), &request(Some(thread_binding()), None))
            .await
            .unwrap();

        assert_eq!(
            stub.paths(),
            vec!["/agents.conversations.create", "/conversations.invite"]
        );
        assert_eq!(
            body(&stub, "/agents.conversations.create"),
            serde_json::json!({
                "name": "SP-1234",
                "session_id": "task-1",
                "origin_channel_id": "C0ASK",
                "origin_message_ts": "1700000000.000100",
                "is_private": true,
            })
        );
        assert_eq!(
            body(&stub, "/conversations.invite"),
            serde_json::json!({ "channel": "C0ROOM", "users": "U0PERSON" })
        );
        assert_eq!(opened.destination.subject_kind, "slack_channel");
        assert_eq!(opened.destination.subject_key, "T1:C0ROOM");
    }

    #[tokio::test]
    async fn a_room_with_only_an_anchor_opens_from_its_own_announcement() {
        let stub = slack().await;

        let opened = adapter(&stub)
            .open_room(&connection(), &request(None, Some("channel:C0FACTORY")))
            .await
            .unwrap();

        assert_eq!(body(&stub, "/chat.postMessage")["channel"], "C0FACTORY");
        let create = body(&stub, "/agents.conversations.create");
        assert_eq!(create["origin_channel_id"], "C0FACTORY");
        assert_eq!(create["origin_message_ts"], "1700000000.000900");
        assert_eq!(opened.destination.subject_key, "T1:C0ROOM");
    }

    #[tokio::test]
    async fn a_failed_invite_still_opens_the_room() {
        let stub = ProviderStub::start(|path| match path {
            "/agents.conversations.create" => {
                serde_json::json!({ "ok": true, "channel_id": "C0ROOM" })
            }
            _ => serde_json::json!({ "ok": false, "error": "missing_scope" }),
        })
        .await;

        let opened = adapter(&stub)
            .open_room(&connection(), &request(Some(thread_binding()), None))
            .await
            .unwrap();

        assert_eq!(opened.destination.subject_key, "T1:C0ROOM");
    }

    #[tokio::test]
    async fn a_room_slack_refuses_fails_permanently() {
        let stub = ProviderStub::start(
            |_| serde_json::json!({ "ok": false, "error": "feature_disabled" }),
        )
        .await;

        let err = adapter(&stub)
            .open_room(&connection(), &request(Some(thread_binding()), None))
            .await
            .unwrap_err();

        let failure = moatless_adapter_model::delivery::AdapterDeliveryFailure::from(err);
        assert!(failure.is_permanent());
    }

    /// Answers `is_bound` for `room` and `is_actively_bound` for `held`.
    struct Bound {
        room: &'static str,
        held: Option<&'static str>,
    }

    #[async_trait::async_trait]
    impl moatless_adapter_model::binding::BindingProbe for Bound {
        async fn is_bound(
            &self,
            _adapter_kind: &str,
            _subject_kind: &str,
            subject_key: &str,
        ) -> Result<bool, moatless_types::store::StoreError> {
            Ok(subject_key == self.room)
        }

        async fn is_actively_bound(
            &self,
            _adapter_kind: &str,
            _subject_kind: &str,
            subject_key: &str,
        ) -> Result<bool, moatless_types::store::StoreError> {
            Ok(self.held == Some(subject_key))
        }
    }

    async fn route_in_room(
        held: Option<&'static str>,
        thread_ts: Option<&str>,
    ) -> crate::ingress::Routed {
        let stub = slack().await;
        let connections =
            moatless_adapter_model::test_support::InMemoryConnections::new(vec![connection()]);
        let thread = thread_ts.map(|ts| {
            crate::subject::thread_destination(
                "C0ROOM",
                ts,
                &serde_json::json!({ "channel": "C0ROOM", "thread_ts": ts, "team_id": "T1" }),
                None,
            )
        });
        super::route(
            &connections,
            &Bound {
                room: "T1:C0ROOM",
                held,
            },
            &adapter(&stub),
            "T1",
            "C0ROOM",
            thread.as_ref(),
        )
        .await
        .expect("probes answer")
        .expect("C0ROOM is a room")
    }

    #[tokio::test]
    async fn a_message_in_a_thread_another_task_holds_goes_to_that_thread() {
        let routed = route_in_room(
            Some("T1:C0ROOM:1700000000.000100"),
            Some("1700000000.000100"),
        )
        .await;

        assert_eq!(routed.destination.subject_kind, "slack_thread");
        assert_eq!(
            routed.destination.subject_key,
            "T1:C0ROOM:1700000000.000100"
        );
    }

    #[tokio::test]
    async fn a_message_in_an_unheld_thread_goes_to_the_room() {
        let routed = route_in_room(
            Some("T1:C0ROOM:1700000000.000999"),
            Some("1700000000.000100"),
        )
        .await;

        assert_eq!(routed.destination.subject_key, "T1:C0ROOM");
    }

    #[tokio::test]
    async fn a_top_level_message_goes_to_the_room() {
        let routed = route_in_room(Some("T1:C0ROOM:1700000000.000100"), None).await;

        assert_eq!(routed.destination.subject_key, "T1:C0ROOM");
    }
}
