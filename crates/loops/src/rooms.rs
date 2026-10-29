//! Task Rooms: the conversation space a Loop's `room` key opens for each Task
//! it spawns, bound to the Task with [`ROOM_BINDING_ORIGIN`].
//!
//! [`RoomOpeningTaskOpener`] admits one durable [`ROOM_OPEN_KIND`] delivery per
//! Task, so every Loop path gets a room without naming it; [`TaskRooms`]
//! executes it through the hosting Adapter's [`AdapterTaskRoom`].
//!
//! [`AdapterTaskRoom`]: moatless_adapter_model::capabilities::AdapterTaskRoom

use std::sync::Arc;

use moatless_adapter_model::binding::{BindScope, ROOM_BINDING_ORIGIN};
use moatless_adapter_model::capabilities::RoomRequest;
use moatless_adapter_model::connections::ConnectionLookup;
use moatless_adapter_model::delivery::{
    AdapterDelivery, AdapterDeliveryFailure, AdapterDeliveryHandler, AdapterDeliveryRequest,
    DeliveryAdmission,
};
use moatless_adapter_model::registry::Adapters;
use moatless_ports::identity::ProviderIdentityLink;
use moatless_ports::task::{NewTask, TaskOpenError, TaskOpened, TaskOpener};
use moatless_types::adapter::{AdapterConnection, TaskBinding};
use moatless_types::loops::RoomSpec;
use serde::{Deserialize, Serialize};

use crate::bindings::operations::Bindings;

/// The delivery kind that opens one Task's room.
pub const ROOM_OPEN_KIND: &str = "room.open";

/// What [`TaskRooms`] needs to open a room, captured when the Task opens.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RoomOpenPayload {
    task_id: String,
    title: String,
    room: RoomSpec,
    /// User id of who the Task was opened for.
    requester: Option<String>,
    /// User id the Task acts as.
    run_as: Option<String>,
}

/// Opens Tasks through `inner`, then requests the room the Loop declares.
///
/// A failed admission is logged and the Task kept: the room is an addition to
/// a Task that already exists, so failing the open would orphan it.
pub struct RoomOpeningTaskOpener {
    inner: Arc<dyn TaskOpener>,
    admission: Arc<dyn DeliveryAdmission>,
}

impl RoomOpeningTaskOpener {
    pub fn new(inner: Arc<dyn TaskOpener>, admission: Arc<dyn DeliveryAdmission>) -> Self {
        Self { inner, admission }
    }
}

#[async_trait::async_trait]
impl TaskOpener for RoomOpeningTaskOpener {
    async fn open(&self, req: NewTask) -> Result<TaskOpened, TaskOpenError> {
        let Some(room) = req.room.clone() else {
            return self.inner.open(req).await;
        };
        let title = req
            .name
            .clone()
            .or_else(|| req.description.clone())
            .unwrap_or_else(|| "task".to_string());
        let requester = req.created_by.clone();
        let run_as = req.run_as_user_id.clone().or_else(|| requester.clone());

        let opened = self.inner.open(req).await?;
        let payload = RoomOpenPayload {
            task_id: opened.task_id.clone(),
            title,
            requester,
            run_as,
            room,
        };
        let payload_json = match serde_json::to_value(&payload) {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!(task_id = %opened.task_id, error = %e, "Task Room not requested");
                return Ok(opened);
            }
        };
        let request = AdapterDeliveryRequest {
            adapter: payload.room.via.as_str(),
            kind: ROOM_OPEN_KIND,
            event_id: opened.task_id.clone(),
            payload: payload_json,
        };
        if let Err(e) = self.admission.admit(request).await {
            tracing::warn!(task_id = %opened.task_id, error = %e, "Task Room not requested");
        }
        Ok(opened)
    }
}

/// Executes [`ROOM_OPEN_KIND`]: opens the room and binds the Task to it.
pub struct TaskRooms {
    adapters: Arc<Adapters>,
    connections: Arc<dyn ConnectionLookup>,
    bindings: Arc<Bindings>,
    identities: Arc<dyn ProviderIdentityLink>,
    /// Without a trailing slash; empty when none is configured.
    frontend_url: String,
}

impl TaskRooms {
    pub fn new(
        adapters: Arc<Adapters>,
        connections: Arc<dyn ConnectionLookup>,
        bindings: Arc<Bindings>,
        identities: Arc<dyn ProviderIdentityLink>,
        frontend_url: &str,
    ) -> Self {
        Self {
            adapters,
            connections,
            bindings,
            identities,
            frontend_url: frontend_url.trim_end_matches('/').to_string(),
        }
    }

    /// `spec.connection` when named, else the origin's, else `via`'s only one.
    async fn connection(
        &self,
        spec: &RoomSpec,
        origin: Option<&TaskBinding>,
    ) -> Result<AdapterConnection, AdapterDeliveryFailure> {
        let via = spec.via.as_str();
        let named = spec
            .connection_id
            .as_deref()
            .or(origin.map(|b| b.connection_id.as_str()));
        let found = if let Some(id) = named {
            self.connections
                .by_id_trusted(id)
                .await
                .map_err(AdapterDeliveryFailure::retryable)?
                .filter(|c| c.adapter_kind == spec.via)
        } else {
            let mut all = self
                .connections
                .by_provider(via)
                .await
                .map_err(AdapterDeliveryFailure::retryable)?;
            (all.len() == 1).then(|| all.remove(0))
        };
        found.ok_or_else(|| {
            AdapterDeliveryFailure::permanent(RoomError(format!(
                "no single {via} connection to open the room on; name one with `connection`"
            )))
        })
    }

    /// Provider ids of `spec.invite`, skipping whoever has no `via` account.
    async fn invitees(&self, payload: &RoomOpenPayload) -> Vec<String> {
        let via = payload.room.via.as_str();
        let mut ids = Vec::new();
        for invitee in &payload.room.invite {
            let resolved = match invitee.as_str() {
                "requester" => self.for_user(via, payload.requester.as_deref()).await,
                "run_as" | "runAs" => self.for_user(via, payload.run_as.as_deref()).await,
                login => self.identities.provider_id_for_login(via, login).await,
            };
            match resolved {
                Ok(Some(id)) if !ids.contains(&id) => ids.push(id),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, ?invitee, "Task Room invitee unresolved"),
            }
        }
        ids
    }

    async fn for_user(
        &self,
        via: &str,
        user_id: Option<&str>,
    ) -> Result<Option<String>, moatless_types::store::StoreError> {
        let Some(user_id) = user_id.and_then(|u| u.parse::<uuid::Uuid>().ok()) else {
            return Ok(None);
        };
        self.identities.provider_id_for_user(via, &user_id).await
    }
}

#[async_trait::async_trait]
impl AdapterDeliveryHandler for TaskRooms {
    async fn execute(&self, delivery: &AdapterDelivery) -> Result<(), AdapterDeliveryFailure> {
        let payload: RoomOpenPayload = serde_json::from_value(delivery.payload.clone())
            .map_err(AdapterDeliveryFailure::permanent)?;
        let via = payload.room.via;
        let host = self.adapters.task_room(via)?;

        let bindings = self
            .bindings
            .for_task(&payload.task_id)
            .await
            .map_err(AdapterDeliveryFailure::retryable)?;
        let on_via = |b: &&TaskBinding| b.adapter_kind == via && b.active;
        if bindings
            .iter()
            .filter(on_via)
            .any(|b| b.origin == ROOM_BINDING_ORIGIN)
        {
            return Ok(());
        }
        let origin = bindings.iter().find(on_via).cloned();
        if origin.is_none() && payload.room.anchor.is_none() {
            // The ingress path binds the triggering thread just after the
            // Task opens, so this delivery can run first.
            return Err(AdapterDeliveryFailure::retryable(RoomError(format!(
                "task {} has no {} binding to open its room from yet",
                payload.task_id,
                via.as_str()
            ))));
        }

        let connection = self.connection(&payload.room, origin.as_ref()).await?;
        let request = RoomRequest {
            task_id: payload.task_id.clone(),
            title: payload.title.clone(),
            origin,
            anchor: payload.room.anchor.clone(),
            invitees: self.invitees(&payload).await,
            private: payload.room.private,
            task_url: (!self.frontend_url.is_empty())
                .then(|| format!("{}/tasks/{}", self.frontend_url, payload.task_id)),
        };
        let opened = host.open_room(&connection, &request).await?;

        let destination = opened.destination;
        self.bindings
            .bind(
                &connection,
                None,
                &payload.task_id,
                via.as_str(),
                destination.subject_kind,
                &destination.subject_key,
                destination.destination_config,
                ROOM_BINDING_ORIGIN,
                destination.last_external_event_id,
                &BindScope::default(),
            )
            .await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct RoomError(String);

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use moatless_adapter_model::delivery::{Admission, AdmitError};
    use moatless_ports::task::{MockTaskOpener, SandboxProvision};
    use moatless_types::adapter::AdapterKind;
    use moatless_types::agent::AgentProfile;
    use moatless_types::task::Visibility;

    use super::*;

    #[derive(Default)]
    struct Admitted(Mutex<Vec<AdapterDeliveryRequest>>);

    #[async_trait::async_trait]
    impl DeliveryAdmission for Admitted {
        async fn admit(&self, request: AdapterDeliveryRequest) -> Result<Admission, AdmitError> {
            self.0.lock().unwrap().push(request);
            Ok(Admission::Admitted)
        }
    }

    pub(super) fn room() -> RoomSpec {
        RoomSpec {
            via: AdapterKind::Slack,
            connection_id: None,
            anchor: Some("channel:C0FACTORY".into()),
            invite: vec!["requester".into()],
            private: Some(true),
        }
    }

    fn new_task(room: Option<RoomSpec>) -> NewTask {
        NewTask {
            workspace_id: "ws-1".into(),
            agent_profile: AgentProfile::ClaudeCode,
            model: None,
            name: Some("SP-1234".into()),
            description: Some("Build it".into()),
            created_by: Some("person".into()),
            run_as_user_id: Some("bot".into()),
            visibility: Visibility::Private,
            loop_id: "loop-1".into(),
            append_system_prompt: None,
            ttl_minutes: 60,
            close_after_settled_days: 7,
            settle_after_hours: None,
            node_selector: None,
            stop_when_done: false,
            tag_ids: Vec::new(),
            skills: Vec::new(),
            memory_preload: Vec::new(),
            related_to: None,
            event_metadata: serde_json::Value::Null,
            room,
        }
    }

    fn opener() -> (RoomOpeningTaskOpener, Arc<Admitted>) {
        let mut inner = MockTaskOpener::new();
        inner.expect_open().returning(|_| {
            Ok(TaskOpened {
                task_id: "task-1".into(),
                provision: SandboxProvision::Started,
            })
        });
        let admitted = Arc::new(Admitted::default());
        (
            RoomOpeningTaskOpener::new(Arc::new(inner), admitted.clone()),
            admitted,
        )
    }

    #[tokio::test]
    async fn a_task_opened_with_a_room_requests_it_once_keyed_by_the_task() {
        let (opener, admitted) = opener();

        opener.open(new_task(Some(room()))).await.unwrap();

        let admitted = admitted.0.lock().unwrap();
        assert_eq!(admitted.len(), 1);
        assert_eq!(admitted[0].kind, ROOM_OPEN_KIND);
        assert_eq!(admitted[0].adapter, "slack");
        assert_eq!(admitted[0].event_id, "task-1");
        let payload: RoomOpenPayload = serde_json::from_value(admitted[0].payload.clone()).unwrap();
        assert_eq!(payload.title, "SP-1234");
        assert_eq!(payload.requester.as_deref(), Some("person"));
        assert_eq!(payload.run_as.as_deref(), Some("bot"));
        assert_eq!(payload.room, room());
    }

    #[tokio::test]
    async fn a_task_opened_without_a_room_requests_none() {
        let (opener, admitted) = opener();

        opener.open(new_task(None)).await.unwrap();

        assert!(admitted.0.lock().unwrap().is_empty());
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "rooms_db_tests.rs"]
mod db_tests;
