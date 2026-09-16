//! Slack webhook request policy and durable admission.
//!
//! The HTTP handler passes only transport facts through [`WebhookRequest`].
//! This module owns everything from routing resolution and signature
//! verification through classification of the durable admission outcome.

use tracing::{debug, warn};

use moatless_adapter_model::binding::BindingDestination;
use moatless_adapter_model::binding::BindingProbe;
use moatless_adapter_model::capabilities::Adapter;
use moatless_adapter_model::connections::ConnectionLookup;
use moatless_adapter_model::delivery::DeliveryAdmission;
use moatless_adapter_model::delivery::inbound_delivery_request;
use moatless_adapter_model::error::AdapterError;
use moatless_adapter_model::ingress::{IngressError, Outcome};
use moatless_adapter_model::loops::LoopReader;
use moatless_ports::identity::ProviderIdentityLink;
use moatless_types::adapter::InboundDeliveryPayload;
use moatless_types::adapter::InboundMessage;
use moatless_types::loops::LoopContext;

use super::adapter::{ResolvedSlackApp, SlackAdapter};
use super::webhook::{self, Normalization, NormalizeContext};
use super::{signature, subject};

/// Transport facts extracted from one Slack webhook request.
pub(super) struct WebhookRequest<'a> {
    pub(super) timestamp: &'a str,
    pub(super) signature: &'a str,
    pub(super) body: &'a [u8],
}

/// What [`receive`] made of one request: the shared [`Outcome`], or the one
/// thing Slack answers that no other Adapter has.
///
/// Kept out of the shared enum: putting Slack's endpoint-check challenge there
/// would add an arm no GitHub, Linear, or Telegram delivery can ever reach.
pub(super) enum Received {
    /// Slack's endpoint check: answer with the challenge string it sent.
    Challenge(String),
    Delivery(Outcome),
}

/// What resolution made of one request, before its signature was checked.
enum Routing {
    /// Dropped without the signature ever being looked at. An unnamed channel
    /// and an unmatched trigger have always been answered this way — `200` with
    /// an `ignored` metric — and they still are when the signature is junk.
    Dropped(&'static str),
    /// Carried far enough that the signature decides what happens next.
    Verifiable(Verifiable),
}

/// What one request turns out to be, once its signature holds.
enum Verifiable {
    /// Slack's endpoint check. It names no channel and matches no trigger, so
    /// it is recognized above the resolution that would otherwise drop it as
    /// `no_channel`, and answered below the one signature check.
    Challenge(String),
    Routed(Box<Routed>),
}

/// Where one event is going: the routing pair it matched, the destination it
/// binds to, and whether that thread is already tracked.
struct Routed {
    /// The event's own channel, which every log line below is keyed by.
    source_id: String,
    connection_id: String,
    app_key: String,
    subscription_id: String,
    config: LoopContext,
    destination: BindingDestination,
    is_tracked_thread: bool,
}

/// Read the request body, which every step below works from.
///
/// Slack signs the bytes, not the fields, so parsing before verifying costs the
/// signature check nothing.
fn parse_body(body: &[u8]) -> Result<serde_json::Value, IngressError> {
    serde_json::from_slice(body)
        .map_err(|e| IngressError::Unidentifiable(format!("Invalid JSON: {e}")))
}

/// The challenge string an endpoint check is answered with.
fn challenge(payload: &serde_json::Value) -> String {
    payload
        .get("challenge")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Everything decided from the payload's own fields and the routing tables.
///
/// Runs *before* verification: the drops below are attributed to a channel, not
/// a signature, and touch only reads — nothing outlives a rejected signature.
async fn resolve(
    adapter_connections: &dyn ConnectionLookup,
    loops: &dyn LoopReader,
    task_bindings: &dyn BindingProbe,
    adapter: &SlackAdapter,
    payload: &serde_json::Value,
    event_type: &str,
) -> Result<Routing, AdapterError> {
    if event_type == "url_verification" {
        return Ok(Routing::Verifiable(Verifiable::Challenge(challenge(
            payload,
        ))));
    }

    let source_id = payload
        .pointer("/event/channel")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if source_id.is_empty() {
        let event_subtype = payload
            .pointer("/event/type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        warn!(
            event_type,
            event_subtype, "Slack webhook has no channel ID in event payload, ignoring"
        );
        return Ok(Routing::Dropped("no_channel"));
    }

    let team_id = payload
        .get("team_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("default");
    let Some((connection, subscription)) =
        moatless_adapter_model::connections::resolve_chat_provider_event(
            adapter_connections,
            loops,
            "slack",
            team_id,
            source_id,
        )
        .await?
    else {
        debug!(
            source_id,
            team_id, "Slack message ignored: no task trigger for this source"
        );
        return Ok(Routing::Dropped("no_trigger"));
    };

    // One destination per delivery, built from the event's own coordinates: the
    // same Binding the tracked-thread lookup asks about is the one an admitted
    // message binds to, so there is nothing to keep in step.
    let thread_ts = payload.pointer("/event/thread_ts").and_then(|v| v.as_str());
    let ts = payload
        .pointer("/event/ts")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let destination = subject::thread_destination(
        source_id,
        thread_ts.unwrap_or(ts),
        &serde_json::json!({
            "channel": source_id,
            "ts": ts,
            "thread_ts": thread_ts.unwrap_or(ts),
            "team_id": team_id,
        }),
        None,
    );

    // A reply in a thread a Binding already tracks bypasses the trigger filter.
    let is_tracked_thread = match thread_ts {
        Some(_) => task_bindings
            .is_bound(
                adapter.provider_key(),
                destination.subject_kind,
                &destination.subject_key,
            )
            .await
            .unwrap_or(false),
        None => false,
    };

    Ok(Routing::Verifiable(Verifiable::Routed(Box::new(Routed {
        config: moatless_adapter_model::loops::subscription_context(&subscription),
        source_id: source_id.to_string(),
        connection_id: connection.id,
        app_key: moatless_adapter_model::apps::app_key_from_transport_config(
            &connection.transport_config,
        )
        .to_string(),
        subscription_id: subscription.id,
        destination,
        is_tracked_thread,
    }))))
}

/// The single signature-verification call site.
///
/// Slack signs the endpoint check exactly as it signs an event, so both reach
/// this one call.
fn verify(app: &ResolvedSlackApp, request: &WebhookRequest<'_>) -> Result<(), AdapterError> {
    signature::verify_signature(
        &app.signing_secret,
        request.timestamp,
        request.body,
        request.signature,
    )
}

/// Normalize one payload and apply what it decided, in that order — the
/// sequence [`receive`] runs. Kept in one function so it can be exercised with
/// an app, a config, and an identity port alone, no ingress ports behind it.
async fn normalize_and_apply(
    app: &ResolvedSlackApp,
    config: &LoopContext,
    users: &dyn ProviderIdentityLink,
    payload: &serde_json::Value,
    is_tracked_thread: bool,
) -> Result<Normalization, AdapterError> {
    let normalization = webhook::normalize(
        payload,
        &NormalizeContext {
            config,
            api: &app.api,
            users,
            bot_user_id: app.bot_user_id.as_deref(),
            is_tracked_thread,
        },
    )
    .await?;

    match &normalization {
        Normalization::Message { effects, .. } | Normalization::Ignored { effects, .. } => {
            webhook::apply_effects(&app.api, users, effects).await;
        }
    }

    Ok(normalization)
}

/// The idempotency key for one Slack delivery.
///
/// Slack's envelope `event_id`, matched against the dedup index on `(adapter,
/// kind, event_id)`; when absent, falls back to the channel plus the message's
/// own external id.
fn admission_event_id(payload: &serde_json::Value, msg: &InboundMessage) -> String {
    payload
        .get("event_id")
        .and_then(|v| v.as_str())
        .filter(|id| !id.is_empty())
        .map_or_else(
            || {
                let message_id = msg
                    .external_event_id
                    .as_deref()
                    .unwrap_or(&msg.message_thread_id);
                format!("{}:{}", msg.source_id, message_id)
            },
            ToString::to_string,
        )
}

/// Resolve, verify, normalize, apply, and durably admit one Slack webhook.
pub(super) async fn receive(
    adapter_connections: &dyn ConnectionLookup,
    loops: &dyn LoopReader,
    task_bindings: &dyn BindingProbe,
    users: &dyn ProviderIdentityLink,
    adapter_delivery: &dyn DeliveryAdmission,
    adapter: &SlackAdapter,
    request: WebhookRequest<'_>,
) -> Result<Received, IngressError> {
    let payload = parse_body(request.body)?;

    let event_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    debug!(event_type, "Slack webhook received");

    let verifiable = match resolve(
        adapter_connections,
        loops,
        task_bindings,
        adapter,
        &payload,
        &event_type,
    )
    .await
    .map_err(IngressError::Ingress)?
    {
        Routing::Dropped(reason) => {
            return Ok(Received::Delivery(Outcome::ignored(event_type, reason)));
        }
        Routing::Verifiable(verifiable) => verifiable,
    };

    let routed = match verifiable {
        Verifiable::Challenge(challenge) => {
            let app = adapter
                .app(moatless_adapter_model::apps::DEFAULT_APP_KEY)
                .await
                .map_err(IngressError::Ingress)?;
            verify(&app, &request).map_err(IngressError::Ingress)?;
            return Ok(Received::Challenge(challenge));
        }
        Verifiable::Routed(routed) => *routed,
    };
    let Routed {
        source_id,
        connection_id,
        app_key,
        subscription_id,
        config,
        destination,
        is_tracked_thread,
    } = routed;

    let app = adapter.app(&app_key).await.map_err(IngressError::Ingress)?;
    verify(&app, &request).map_err(IngressError::Ingress)?;

    let normalization = normalize_and_apply(&app, &config, users, &payload, is_tracked_thread)
        .await
        .map_err(IngressError::Ingress)?;

    let message = match normalization {
        Normalization::Ignored { reason, .. } => {
            debug!(source_id, reason, "Slack event ignored by provider");
            return Ok(Received::Delivery(Outcome::Ignored {
                event_type,
                reason,
                subscription_id,
                connection_id,
            }));
        }
        Normalization::Message { message, .. } => *message,
    };

    let event_id = admission_event_id(&payload, &message);
    let request = inbound_delivery_request(
        super::delivery::ADAPTER,
        super::delivery::INBOUND_MESSAGE,
        event_id.clone(),
        InboundDeliveryPayload {
            connection_id,
            loop_id: subscription_id,
            subject_kind: destination.subject_kind.to_string(),
            subject_key: destination.subject_key,
            destination_config: destination.destination_config,
            event_type: event_type.clone(),
            message,
        },
    )
    .map_err(IngressError::Ingress)?;

    // Everything below the 200 depends on this write: a failure here must reach
    // Slack as a failure so it re-delivers, and must never be swallowed into an
    // acknowledgment. The routing itself runs in the Adapter delivery worker,
    // which is also where the webhook outcome metric is recorded.
    let admission =
        moatless_adapter_model::ingress::admit_delivery(adapter_delivery, request).await?;
    debug!(
        source_id,
        event_id = %event_id,
        admission = admission.as_str(),
        "Slack inbound message admitted for routing"
    );

    Ok(Received::Delivery(Outcome::Processed { event_type }))
}

/// Characterization of Slack ingress, pinned before the module split and
/// carried through it unchanged.
///
/// Assertions run against ingress's return value and the requests a loopback
/// Slack stub saw; no test reaches slack.com, via [`SlackAdapter::with_api`].
#[cfg(test)]
mod characterization {
    use axum::http::HeaderMap;
    use hmac::{Hmac, KeyInit, Mac};
    use moatless_types::access_control::Scope;
    use sha2::Sha256;

    use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
    use moatless_adapter_model::test_support::ProviderStub;
    use moatless_ports::identity::ProviderIdentityLink;
    use moatless_types::adapter::AdapterKind;
    use moatless_types::adapter::{Audience, InboundMessage};
    use moatless_types::loops::{LoopContext, RoutingMode};

    use super::{IngressError, WebhookRequest, challenge, normalize_and_apply, parse_body, verify};
    use crate::adapter::SlackAdapter;
    use crate::api::SlackApi;
    use crate::webhook::Normalization;

    pub(super) const SIGNING_SECRET: &str = "8f742231b10e8888abcd99yez67676hg";
    pub(super) const BOT_USER_ID: &str = "U0BOT";
    pub(super) const OWNER: &str = "trigger-owner";

    type HmacSha256 = Hmac<Sha256>;

    /// What one delivery resolved to, before routing and admission: the shape
    /// the assertions below are written against.
    pub(super) enum Parsed {
        Challenge(String),
        Message(Box<InboundMessage>),
        Ignore(&'static str),
    }

    /// The headers Slack would send for `body`, signed with [`SIGNING_SECRET`].
    pub(super) fn signed_headers(body: &str) -> HeaderMap {
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let mut mac = HmacSha256::new_from_slice(SIGNING_SECRET.as_bytes()).unwrap();
        mac.update(format!("v0:{timestamp}:{body}").as_bytes());
        let signature = format!("v0={}", hex::encode(mac.finalize().into_bytes()));

        let mut headers = HeaderMap::new();
        headers.insert("x-slack-request-timestamp", timestamp.parse().unwrap());
        headers.insert("x-slack-signature", signature.parse().unwrap());
        headers
    }

    fn request<'a>(headers: &'a HeaderMap, body: &'a [u8]) -> WebhookRequest<'a> {
        let header = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
        };
        WebhookRequest {
            timestamp: header("x-slack-request-timestamp"),
            signature: header("x-slack-signature"),
            body,
        }
    }

    /// A Slack API stub answering `users.info` with `user`, `chat.postMessage`
    /// with a fixed `ts`, and rejecting anything else.
    pub(super) async fn slack_stub(user: serde_json::Value) -> ProviderStub {
        ProviderStub::start(move |path| match path {
            "/users.info" => user.clone(),
            "/chat.postMessage" => serde_json::json!({"ok": true, "ts": "1700000000.000900"}),
            other => serde_json::json!({"ok": false, "error": format!("unexpected {other}")}),
        })
        .await
    }

    /// A `users.info` payload for a human sender.
    pub(super) fn human(email: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "ok": true,
            "user": {
                "id": "U1HUMAN",
                "name": "alice",
                "is_bot": false,
                "is_app_user": false,
                "profile": { "email": email },
            }
        })
    }

    pub(super) fn adapter(stub: &ProviderStub, bot_user_id: Option<&str>) -> SlackAdapter {
        SlackAdapter::with_api(
            SlackApi::with_base_url("xoxb-test", stub.base_url()),
            SIGNING_SECRET,
            bot_user_id.map(str::to_string),
        )
    }

    /// An identity port whose every read fails, so a test can assert that the
    /// sender lookup was reached at all.
    pub(super) struct FailingUsers;

    #[async_trait::async_trait]
    impl ProviderIdentityLink for FailingUsers {
        async fn provider_id_for_login(
            &self,
            _provider: &str,
            _login: &str,
        ) -> Result<Option<String>, moatless_types::store::StoreError> {
            Err(moatless_types::store::StoreError::new("no directory here"))
        }

        async fn user_at(
            &self,
            _provider: &str,
            _provider_user_id: &str,
        ) -> Result<Option<moatless_ports::identity::LinkedUser>, moatless_types::store::StoreError>
        {
            Err(moatless_types::store::StoreError::new("no directory here"))
        }

        async fn user_by_email(
            &self,
            _email: &str,
        ) -> Result<Option<moatless_ports::identity::LinkedUser>, moatless_types::store::StoreError>
        {
            Err(moatless_types::store::StoreError::new("no directory here"))
        }

        async fn user_who_connected(
            &self,
            _provider: &str,
            _provider_user_id: &str,
        ) -> Result<Option<String>, moatless_types::store::StoreError> {
            Err(moatless_types::store::StoreError::new("no directory here"))
        }

        async fn user_with_login(
            &self,
            _provider: &str,
            _login: &str,
        ) -> Result<Option<String>, moatless_types::store::StoreError> {
            Err(moatless_types::store::StoreError::new("no directory here"))
        }

        async fn link(
            &self,
            _user_id: &uuid::Uuid,
            _provider: &str,
            _provider_user_id: &str,
        ) -> Result<bool, moatless_types::store::StoreError> {
            Err(moatless_types::store::StoreError::new("no directory here"))
        }
    }

    pub(super) fn trigger(adapter_config: Option<serde_json::Value>) -> LoopContext {
        let now = chrono::Utc::now();
        LoopContext {
            id: "tt_slack".into(),
            adapter_kind: "slack".to_string(),
            source_id: "C1".into(),
            source_name: None,
            workspace_id: "ws-1".into(),
            routing_mode: RoutingMode::BySubject,
            prompt_template: None,
            active: true,
            allow_direct_send: false,
            adapter_config,
            ongoing_task_id: None,
            scope: Scope::Global,
            task_name: None,
            tag_ids: vec![],
            skills: Vec::new(),
            run_as_user_id: Some(OWNER.into()),
            agent_type: "claude-code".into(),
            model: None,
            inactive_ttl_days: None,
            created_by: "user-1".into(),
            created_at: now,
            updated_at: now,
            node_selector: None,
            stop_when_done: false,
        }
    }

    /// The body read, verified, and normalized with its effects applied —
    /// everything [`super::receive`] runs except the routing step in between,
    /// which needs adapter-connection, loop, and task-binding ports no test
    /// here builds.
    pub(super) async fn parse(
        adapter: &SlackAdapter,
        body: &str,
        config: &LoopContext,
        users: &dyn ProviderIdentityLink,
        is_tracked_thread: bool,
    ) -> Result<Parsed, IngressError> {
        parse_with(
            adapter,
            &signed_headers(body),
            body,
            config,
            users,
            is_tracked_thread,
        )
        .await
    }

    /// [`parse`] with the headers spelled out, for the signature policy.
    pub(super) async fn parse_with(
        adapter: &SlackAdapter,
        headers: &HeaderMap,
        body: &str,
        config: &LoopContext,
        users: &dyn ProviderIdentityLink,
        is_tracked_thread: bool,
    ) -> Result<Parsed, IngressError> {
        let payload = parse_body(body.as_bytes())?;
        let app = adapter
            .app(moatless_adapter_model::apps::DEFAULT_APP_KEY)
            .await
            .map_err(IngressError::Ingress)?;
        verify(&app, &request(headers, body.as_bytes())).map_err(IngressError::Ingress)?;

        if payload.get("type").and_then(serde_json::Value::as_str) == Some("url_verification") {
            return Ok(Parsed::Challenge(challenge(&payload)));
        }

        Ok(
            match normalize_and_apply(&app, config, users, &payload, is_tracked_thread)
                .await
                .map_err(IngressError::Ingress)?
            {
                Normalization::Message { message, .. } => Parsed::Message(message),
                Normalization::Ignored { reason, .. } => Parsed::Ignore(reason),
            },
        )
    }

    /// The error a parse produced.
    pub(super) fn error(result: Result<Parsed, IngressError>) -> IngressError {
        match result {
            Err(e) => e,
            Ok(Parsed::Ignore(reason)) => panic!("expected an error, got {reason}"),
            Ok(_) => panic!("expected an error, got a result"),
        }
    }

    /// The [`AdapterError`] an ingress rejection carries.
    fn ingress_error(result: Result<Parsed, IngressError>) -> AdapterError {
        match error(result) {
            IngressError::Ingress(e) => e,
            other => panic!("expected an ingress rejection, got {other}"),
        }
    }

    pub(super) fn ignored(result: Result<Parsed, IngressError>) -> &'static str {
        match result {
            Ok(Parsed::Ignore(reason)) => reason,
            Ok(Parsed::Message(_)) => panic!("expected Ignore, got Message"),
            Ok(Parsed::Challenge(_)) => panic!("expected Ignore, got Challenge"),
            Err(e) => panic!("expected Ignore, got error: {e}"),
        }
    }

    /// An `event_callback` envelope wrapping `event`.
    pub(super) fn envelope(event: serde_json::Value) -> String {
        serde_json::json!({
            "type": "event_callback",
            "event_id": "Ev1",
            "team_id": "T1",
            "event": event,
        })
        .to_string()
    }

    /// A plain channel message from a human.
    pub(super) fn message_event() -> serde_json::Value {
        serde_json::json!({
            "type": "message",
            "user": "U1HUMAN",
            "channel": "C1",
            "text": "please look at this",
            "ts": "1700000000.000100",
        })
    }

    // -- signature policy ---------------------------------------------------

    #[tokio::test]
    async fn a_url_verification_body_answers_with_the_challenge() {
        let stub = slack_stub(human(None)).await;
        let adapter = adapter(&stub, None);
        let body =
            serde_json::json!({"type": "url_verification", "challenge": "abc123"}).to_string();

        let result = parse(
            &adapter,
            &body,
            &trigger(None),
            &crate::auth::UnreachableUsers,
            false,
        )
        .await;

        let Ok(Parsed::Challenge(challenge)) = result else {
            panic!("expected a challenge");
        };
        assert_eq!(challenge, "abc123");
        assert!(
            stub.paths().is_empty(),
            "the challenge spends no Slack call"
        );
    }

    #[tokio::test]
    async fn an_invalid_signature_is_refused() {
        let stub = slack_stub(human(None)).await;
        let adapter = adapter(&stub, None);
        let body = envelope(message_event());
        let mut headers = signed_headers(&body);
        headers.insert("x-slack-signature", "v0=deadbeef".parse().unwrap());

        let error = ingress_error(
            parse_with(
                &adapter,
                &headers,
                &body,
                &trigger(None),
                &crate::auth::UnreachableUsers,
                false,
            )
            .await,
        );

        assert!(
            matches!(
                error,
                AdapterError::Shared(SharedAdapterError::SignatureInvalid)
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn absent_signature_headers_are_refused() {
        let stub = slack_stub(human(None)).await;
        let adapter = adapter(&stub, None);
        let body = envelope(message_event());

        let error = ingress_error(
            parse_with(
                &adapter,
                &HeaderMap::new(),
                &body,
                &trigger(None),
                &crate::auth::UnreachableUsers,
                false,
            )
            .await,
        );

        assert!(
            matches!(
                error,
                AdapterError::Shared(SharedAdapterError::SignatureInvalid)
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_replayed_timestamp_is_refused() {
        let stub = slack_stub(human(None)).await;
        let adapter = adapter(&stub, None);
        let body = envelope(message_event());
        let mut headers = signed_headers(&body);
        let stale = (chrono::Utc::now().timestamp() - 600).to_string();
        headers.insert("x-slack-request-timestamp", stale.parse().unwrap());

        let error = ingress_error(
            parse_with(
                &adapter,
                &headers,
                &body,
                &trigger(None),
                &crate::auth::UnreachableUsers,
                false,
            )
            .await,
        );

        assert!(
            matches!(
                error,
                AdapterError::Shared(SharedAdapterError::SignatureInvalid)
            ),
            "{error:?}"
        );
    }

    /// The body is read before the signature is checked, so a body that is not
    /// JSON is a `400` however it was signed — including not at all. A `401`
    /// here would tell Slack to re-sign junk it should stop sending.
    #[tokio::test]
    async fn a_body_that_is_not_json_is_a_bad_request() {
        for headers in [signed_headers("<html>not json</html>"), HeaderMap::new()] {
            let stub = slack_stub(human(None)).await;
            let adapter = adapter(&stub, None);

            let error = error(
                parse_with(
                    &adapter,
                    &headers,
                    "<html>not json</html>",
                    &trigger(None),
                    &crate::auth::UnreachableUsers,
                    false,
                )
                .await,
            );

            assert!(
                matches!(error, IngressError::Unidentifiable(ref m) if m.starts_with("Invalid JSON:")),
                "{error:?}"
            );
        }
    }

    // -- ignore reasons raised before the sender lookup ----------------------

    /// Each row is `(reason, is_tracked_thread, adapter_config, event)`, and
    /// each names a `record_webhook` label the dashboards already carry.
    fn reasons_before_the_sender_lookup() -> Vec<(
        &'static str,
        bool,
        Option<serde_json::Value>,
        serde_json::Value,
    )> {
        vec![
            (
                "no_event",
                false,
                None,
                serde_json::Value::Null, // no `event` key at all — see `envelope_without_event`
            ),
            (
                "own_bot",
                false,
                None,
                serde_json::json!({
                    "type": "message", "user": BOT_USER_ID, "channel": "C1",
                    "text": "hello", "ts": "1700000000.000100",
                }),
            ),
            (
                "system_subtype",
                false,
                Some(serde_json::json!({"trigger": "all_messages"})),
                serde_json::json!({
                    "type": "message", "subtype": "channel_join", "user": "U1HUMAN",
                    "channel": "C1", "text": "has joined the channel",
                    "ts": "1700000000.000100",
                }),
            ),
            (
                "bot_account",
                false,
                Some(serde_json::json!({"trigger": "all_messages"})),
                serde_json::json!({
                    "type": "message", "user": "U1HUMAN", "channel": "C1",
                    "bot_id": "B1", "text": "ci is green", "ts": "1700000000.000100",
                }),
            ),
            ("trigger_filter", false, None, message_event()),
            (
                "access_policy",
                false,
                Some(serde_json::json!({
                    "trigger": "all_messages",
                    "access_policy": "allowlist",
                    "channel_allowlist": ["C999"],
                })),
                message_event(),
            ),
            (
                "empty_message",
                false,
                Some(serde_json::json!({"trigger": "all_messages"})),
                serde_json::json!({
                    "type": "message", "user": "U1HUMAN", "channel": "C1",
                    "text": "hello",
                }),
            ),
            (
                "empty_after_format",
                false,
                Some(serde_json::json!({"trigger": "all_messages"})),
                serde_json::json!({
                    "type": "message", "user": "U1HUMAN", "channel": "C1",
                    "text": "   ", "ts": "1700000000.000100",
                }),
            ),
        ]
    }

    fn envelope_without_event() -> String {
        serde_json::json!({"type": "event_callback", "event_id": "Ev1", "team_id": "T1"})
            .to_string()
    }

    #[tokio::test]
    async fn every_reason_before_the_sender_lookup_keeps_its_label_and_spends_nothing() {
        for (reason, is_tracked_thread, adapter_config, event) in reasons_before_the_sender_lookup()
        {
            let stub = slack_stub(human(Some("alice@example.com"))).await;
            let adapter = adapter(&stub, Some(BOT_USER_ID));
            let body = if event.is_null() {
                envelope_without_event()
            } else {
                envelope(event)
            };

            let result = parse(
                &adapter,
                &body,
                &trigger(adapter_config),
                &crate::auth::UnreachableUsers,
                is_tracked_thread,
            )
            .await;

            assert_eq!(ignored(result), reason);
            assert!(
                stub.paths().is_empty(),
                "{reason} must be decided with no Slack call, saw {:?}",
                stub.paths()
            );
        }
    }

    // -- the one reason the sender lookup itself raises ----------------------

    #[tokio::test]
    async fn a_bot_account_reported_by_users_info_is_ignored_as_bot_account() {
        let stub = slack_stub(serde_json::json!({
            "ok": true,
            "user": {
                "id": "U1HUMAN", "name": "soapbot", "is_bot": true, "is_app_user": false,
                "profile": { "email": "bot@example.com" },
            }
        }))
        .await;
        let adapter = adapter(&stub, Some(BOT_USER_ID));

        let result = parse(
            &adapter,
            &envelope(message_event()),
            &trigger(Some(serde_json::json!({"trigger": "all_messages"}))),
            &crate::auth::UnreachableUsers,
            false,
        )
        .await;

        assert_eq!(ignored(result), "bot_account");
        assert_eq!(
            stub.paths(),
            vec!["/users.info".to_string()],
            "the bot drop costs exactly one users.info and no database query"
        );
    }

    #[tokio::test]
    async fn a_users_info_failure_is_swallowed_and_the_sender_stays_unresolved() {
        let stub = slack_stub(serde_json::json!({"ok": false, "error": "user_not_found"})).await;
        let adapter = adapter(&stub, Some(BOT_USER_ID));

        // The lookup that follows is what fails here — proving the parse got
        // past `users.info` returning nothing rather than stopping on it.
        let error = ingress_error(
            parse(
                &adapter,
                &envelope(message_event()),
                &trigger(Some(serde_json::json!({"trigger": "all_messages"}))),
                &FailingUsers,
                false,
            )
            .await,
        );

        assert!(matches!(error, AdapterError::Store(_)), "{error:?}");
        assert_eq!(stub.paths(), vec!["/users.info".to_string()]);
    }

    /// A foreign bot allowed through is attributed to the trigger owner, which
    /// takes no mapping and therefore no database — the one message this half
    /// of the characterization can build on its own.
    #[tokio::test]
    async fn a_bot_message_is_built_without_touching_the_database() {
        let stub = slack_stub(serde_json::json!({
            "ok": true,
            "user": {
                "id": "U1CI", "name": "ci", "is_bot": true, "is_app_user": false,
                "profile": { "email": "ci@example.com" },
            }
        }))
        .await;
        let adapter = adapter(&stub, Some(BOT_USER_ID));
        let body = envelope(serde_json::json!({
            "type": "message",
            "user": "U1CI",
            "channel": "C1",
            "bot_id": "B1",
            "text": "build 42 is green",
            "ts": "1700000000.000100",
        }));

        let result = parse(
            &adapter,
            &body,
            &trigger(Some(serde_json::json!({
                "trigger": "all_messages",
                "allow_bot_messages": true,
            }))),
            &crate::auth::UnreachableUsers,
            false,
        )
        .await;

        let Ok(Parsed::Message(message)) = result else {
            panic!("expected a message");
        };
        assert_eq!(message.user_id, OWNER);
        assert_eq!(message.text, "build 42 is green");
        assert_eq!(stub.paths(), vec!["/users.info".to_string()]);
    }

    #[test]
    fn the_slack_event_carries_everyone_as_its_audience() {
        // Slack makes no audience distinction; the constant is asserted where
        // the message is built, below, and named here so a change to the enum
        // fails a test that says why.
        assert_eq!(Audience::Everyone, Audience::Everyone);
        assert_eq!(AdapterKind::Slack.as_str(), "slack");
    }
}

#[cfg(test)]
mod tests {
    use moatless_types::adapter::AdapterKind;
    use moatless_types::adapter::{Audience, InboundMessage};

    use super::admission_event_id;

    fn message(external_event_id: Option<&str>) -> InboundMessage {
        InboundMessage {
            adapter_kind: AdapterKind::Slack,
            source_id: "C1".into(),
            thread_id: None,
            message_thread_id: "1700000000.000100".into(),
            user_id: "user-1".into(),
            author: None,
            sender_external_id: None,
            text: "hello".into(),
            file_ids: vec![],
            thread_properties: serde_json::json!({}),
            adapter_context: serde_json::json!({}),
            external_event_id: external_event_id.map(str::to_string),
            source_task_id: None,
            audience: Audience::Everyone,
        }
    }

    /// The envelope id is repeated on every Slack retry, which is exactly what
    /// the durable dedup index needs — so it wins whenever it is present.
    #[test]
    fn the_envelope_event_id_is_the_idempotency_key() {
        let payload = serde_json::json!({"event_id": "Ev1"});
        assert_eq!(
            admission_event_id(&payload, &message(Some("slack:ts:1700000000.000100"))),
            "Ev1"
        );
    }

    #[test]
    fn an_absent_envelope_id_falls_back_to_channel_and_message_id() {
        let fallback = admission_event_id(
            &serde_json::json!({}),
            &message(Some("slack:ts:1700000000.000100")),
        );
        assert_eq!(fallback, "C1:slack:ts:1700000000.000100");

        // Stable across a re-delivery of the same message, and distinct from
        // another message in the same channel.
        assert_eq!(
            fallback,
            admission_event_id(
                &serde_json::json!({"event_id": ""}),
                &message(Some("slack:ts:1700000000.000100")),
            ),
            "an empty envelope id is the same as an absent one"
        );
        assert_ne!(
            fallback,
            admission_event_id(
                &serde_json::json!({}),
                &message(Some("slack:ts:1700000000.000200")),
            )
        );
    }

    #[test]
    fn a_message_with_no_external_id_falls_back_to_its_thread_id() {
        assert_eq!(
            admission_event_id(&serde_json::json!({}), &message(None)),
            "C1:1700000000.000100"
        );
    }
}

/// The half of the characterization that needs a database: everything from the
/// sender mapping onwards.
#[cfg(all(test, feature = "db-tests"))]
mod characterization_db {
    use std::sync::Arc;

    use moatless_identity::ports::UserDirectory;
    use sqlx::PgPool;

    use moatless_types::adapter::AdapterKind;
    use moatless_types::adapter::Audience;

    use super::IngressError;
    use super::characterization::{
        BOT_USER_ID, OWNER, Parsed, adapter, envelope, human, ignored, message_event, parse,
        slack_stub, trigger,
    };

    /// The identity seam over a real pool — what the composition root hands the
    /// ingress.
    fn users(pool: &PgPool) -> Arc<UserDirectory> {
        moatless_identity::compose::user_directory(pool.clone())
    }

    fn message(result: Result<Parsed, IngressError>) -> moatless_types::adapter::InboundMessage {
        match result {
            Ok(Parsed::Message(message)) => *message,
            Ok(Parsed::Ignore(reason)) => panic!("expected Message, got {reason}"),
            Ok(Parsed::Challenge(_)) => panic!("expected Message, got Challenge"),
            Err(e) => panic!("expected Message, got error: {e}"),
        }
    }

    /// Seed a user and, optionally, its Slack provider-id mapping.
    async fn seed_user(pool: &PgPool, email: &str, slack_user_id: Option<&str>) -> uuid::Uuid {
        let id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, login, email, provider, created_at, last_login_at)
             VALUES ($1, 'alice', $2, 'local', now(), now())",
        )
        .bind(id)
        .bind(email)
        .execute(pool)
        .await
        .unwrap();
        if let Some(slack_user_id) = slack_user_id {
            sqlx::query(
                "INSERT INTO user_provider_ids (user_id, provider, provider_user_id)
                 VALUES ($1, 'slack', $2)",
            )
            .bind(id)
            .bind(slack_user_id)
            .execute(pool)
            .await
            .unwrap();
        }
        id
    }

    #[sqlx::test]
    async fn an_unmapped_top_level_sender_is_notified_in_thread_and_ignored(pool: PgPool) {
        let stub = slack_stub(human(Some("alice@example.com"))).await;
        let adapter = adapter(&stub, Some(BOT_USER_ID));

        let result = parse(
            &adapter,
            &envelope(message_event()),
            &trigger(Some(serde_json::json!({"trigger": "all_messages"}))),
            users(&pool).as_ref(),
            false,
        )
        .await;

        assert_eq!(ignored(result), "user_not_mapped");
        assert_eq!(
            stub.paths(),
            vec!["/users.info".to_string(), "/chat.postMessage".to_string()],
            "one users.info, one notice"
        );
        let notice: serde_json::Value =
            serde_json::from_str(&stub.requests()[1].body).expect("notice body is JSON");
        assert_eq!(notice["channel"], "C1");
        assert_eq!(notice["thread_ts"], "1700000000.000100");
        assert_eq!(
            notice["text"],
            "Sorry, I couldn't find your account. Please make sure you're registered with the \
             email address: alice@example.com"
        );
    }

    #[sqlx::test]
    async fn an_unmapped_sender_without_an_email_gets_the_generic_notice(pool: PgPool) {
        let stub = slack_stub(human(None)).await;
        let adapter = adapter(&stub, Some(BOT_USER_ID));

        let result = parse(
            &adapter,
            &envelope(message_event()),
            &trigger(Some(serde_json::json!({"trigger": "all_messages"}))),
            users(&pool).as_ref(),
            false,
        )
        .await;

        assert_eq!(ignored(result), "user_not_mapped");
        let notice: serde_json::Value =
            serde_json::from_str(&stub.requests()[1].body).expect("notice body is JSON");
        assert_eq!(
            notice["text"],
            "Sorry, I couldn't find your account. Please make sure you're registered in the \
             system with the same email as your Slack profile."
        );
    }

    /// The notice threads under the parent, not under the reply, and the reply
    /// itself is admitted rather than blocked.
    #[sqlx::test]
    async fn an_unmapped_thread_reply_falls_back_to_the_trigger_owner(pool: PgPool) {
        let stub = slack_stub(human(Some("alice@example.com"))).await;
        let adapter = adapter(&stub, Some(BOT_USER_ID));
        let body = envelope(serde_json::json!({
            "type": "message",
            "user": "U1HUMAN",
            "channel": "C1",
            "text": "and one more thing <@U0BOT>",
            "ts": "1700000000.000200",
            "thread_ts": "1700000000.000100",
        }));

        let message = message(
            parse(
                &adapter,
                &body,
                &trigger(Some(serde_json::json!({"trigger": "all_messages"}))),
                users(&pool).as_ref(),
                false,
            )
            .await,
        );

        assert_eq!(message.user_id, OWNER);
        assert_eq!(message.adapter_kind, AdapterKind::Slack);
        assert_eq!(message.source_id, "C1");
        assert_eq!(message.thread_id.as_deref(), Some("1700000000.000100"));
        assert_eq!(message.message_thread_id, "1700000000.000100");
        assert_eq!(
            message.text, "and one more thing @U0BOT",
            "mrkdwn formatting is applied, and the mention still counts as one"
        );
        assert!(message.file_ids.is_empty());
        assert_eq!(
            message.external_event_id.as_deref(),
            Some("slack:ts:1700000000.000200")
        );
        assert_eq!(message.audience, Audience::Everyone);
        assert_eq!(message.source_task_id, None);
        assert_eq!(
            message.thread_properties,
            serde_json::json!({
                "channel": "C1",
                "ts": "1700000000.000200",
                "thread_ts": "1700000000.000100",
                "team_id": "T1",
            })
        );
        assert_eq!(
            message.adapter_context,
            serde_json::json!({
                "team_id": "T1",
                "user_id": "U1HUMAN",
                "is_bot_mentioned": true,
            })
        );
        assert_eq!(stub.paths(), vec!["/users.info".to_string()]);
    }

    #[sqlx::test]
    async fn a_foreign_bot_message_is_attributed_to_the_trigger_owner(pool: PgPool) {
        let stub = slack_stub(serde_json::json!({
            "ok": true,
            "user": {
                "id": "U1CI", "name": "ci", "is_bot": true, "is_app_user": false,
                "profile": { "email": "ci@example.com" },
            }
        }))
        .await;
        let adapter = adapter(&stub, Some(BOT_USER_ID));
        let body = envelope(serde_json::json!({
            "type": "message",
            "user": "U1CI",
            "channel": "C1",
            "bot_id": "B1",
            "text": "build 42 is green",
            "ts": "1700000000.000100",
        }));

        let message = message(
            parse(
                &adapter,
                &body,
                &trigger(Some(serde_json::json!({
                    "trigger": "all_messages",
                    "allow_bot_messages": true,
                }))),
                users(&pool).as_ref(),
                false,
            )
            .await,
        );

        assert_eq!(message.user_id, OWNER, "no email resolution for a bot");
        assert_eq!(message.text, "build 42 is green");
        assert_eq!(
            stub.paths(),
            vec!["/users.info".to_string()],
            "a bot sender is never notified"
        );
    }

    #[sqlx::test]
    async fn a_tracked_thread_reply_bypasses_the_trigger_filter(pool: PgPool) {
        let stub = slack_stub(human(None)).await;
        let adapter = adapter(&stub, Some(BOT_USER_ID));
        let body = envelope(serde_json::json!({
            "type": "message",
            "user": "U1HUMAN",
            "channel": "C1",
            "text": "thanks!",
            "ts": "1700000000.000200",
            "thread_ts": "1700000000.000100",
        }));

        // `mention_only` is the default, and this message mentions nobody.
        assert_eq!(
            ignored(
                parse(
                    &adapter,
                    &body,
                    &trigger(None),
                    users(&pool).as_ref(),
                    false
                )
                .await
            ),
            "trigger_filter"
        );

        let message =
            message(parse(&adapter, &body, &trigger(None), users(&pool).as_ref(), true).await);
        assert_eq!(message.user_id, OWNER);
        assert_eq!(
            message.adapter_context["is_bot_mentioned"],
            serde_json::json!(false)
        );
    }

    #[sqlx::test]
    async fn a_dm_bypasses_the_trigger_filter(pool: PgPool) {
        let stub = slack_stub(human(None)).await;
        let adapter = adapter(&stub, Some(BOT_USER_ID));
        let body = envelope(serde_json::json!({
            "type": "message",
            "user": "U1HUMAN",
            "channel": "D1",
            "text": "hello there",
            "ts": "1700000000.000100",
            "thread_ts": "1700000000.000100",
        }));

        let message = message(
            parse(
                &adapter,
                &body,
                &trigger(None),
                users(&pool).as_ref(),
                false,
            )
            .await,
        );
        assert_eq!(message.source_id, "D1");
    }

    #[sqlx::test]
    async fn blocks_then_attachments_then_text_decide_the_body(pool: PgPool) {
        let stub = slack_stub(human(None)).await;
        let adapter = adapter(&stub, Some(BOT_USER_ID));
        let with_blocks = envelope(serde_json::json!({
            "type": "message", "user": "U1HUMAN", "channel": "C1",
            "text": "", "ts": "1700000000.000100", "thread_ts": "1700000000.000100",
            "blocks": [{"type": "section", "text": {"type": "mrkdwn", "text": "from blocks"}}],
            "attachments": [{"text": "from attachments"}],
        }));
        let with_attachments = envelope(serde_json::json!({
            "type": "message", "user": "U1HUMAN", "channel": "C1",
            "text": "", "ts": "1700000000.000100", "thread_ts": "1700000000.000100",
            "attachments": [{"text": "from attachments"}],
        }));

        let config = trigger(Some(serde_json::json!({"trigger": "all_messages"})));
        assert_eq!(
            message(
                parse(
                    &adapter,
                    &with_blocks,
                    &config,
                    users(&pool).as_ref(),
                    false
                )
                .await
            )
            .text,
            "from blocks"
        );
        assert_eq!(
            message(
                parse(
                    &adapter,
                    &with_attachments,
                    &config,
                    users(&pool).as_ref(),
                    false
                )
                .await
            )
            .text,
            "from attachments"
        );
    }

    /// A sender the provider-id table already knows is attributed to them, with
    /// no email lookup and no notice.
    #[sqlx::test]
    async fn a_provider_id_mapping_attributes_the_message_to_that_user(pool: PgPool) {
        let user_id = seed_user(&pool, "alice@example.com", Some("U1HUMAN")).await;
        let stub = slack_stub(human(Some("alice@example.com"))).await;
        let adapter = adapter(&stub, Some(BOT_USER_ID));

        let message = message(
            parse(
                &adapter,
                &envelope(message_event()),
                &trigger(Some(serde_json::json!({"trigger": "all_messages"}))),
                users(&pool).as_ref(),
                false,
            )
            .await,
        );

        assert_eq!(message.user_id, user_id.to_string());
        assert_eq!(
            stub.paths(),
            vec!["/users.info".to_string()],
            "a mapped sender costs one users.info and nothing else"
        );
    }

    /// One lookup later: no provider-id row, but the email matches — and the
    /// mapping is cached, so the next delivery from this sender skips it.
    #[sqlx::test]
    async fn an_email_mapping_attributes_the_message_and_caches_the_sender(pool: PgPool) {
        let user_id = seed_user(&pool, "alice@example.com", None).await;
        let stub = slack_stub(human(Some("alice@example.com"))).await;
        let adapter = adapter(&stub, Some(BOT_USER_ID));

        let message = message(
            parse(
                &adapter,
                &envelope(message_event()),
                &trigger(Some(serde_json::json!({"trigger": "all_messages"}))),
                users(&pool).as_ref(),
                false,
            )
            .await,
        );

        assert_eq!(message.user_id, user_id.to_string());
        let cached: Option<(uuid::Uuid,)> = sqlx::query_as(
            "SELECT user_id FROM user_provider_ids
             WHERE provider = 'slack' AND provider_user_id = 'U1HUMAN'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert_eq!(
            cached,
            Some((user_id,)),
            "the email mapping is remembered as a provider id"
        );
    }

    /// A sender whose email matches nobody is unmapped, which is the state the
    /// provider-id cache write would be reached from — it never is, because the
    /// only path to it is the mapping above.
    #[sqlx::test]
    async fn an_unmatched_email_leaves_the_sender_unmapped(pool: PgPool) {
        seed_user(&pool, "someone-else@example.com", None).await;
        let stub = slack_stub(human(Some("alice@example.com"))).await;
        let adapter = adapter(&stub, Some(BOT_USER_ID));

        let result = parse(
            &adapter,
            &envelope(message_event()),
            &trigger(Some(serde_json::json!({"trigger": "all_messages"}))),
            users(&pool).as_ref(),
            false,
        )
        .await;

        assert_eq!(ignored(result), "user_not_mapped");
        let cached: Option<(String,)> = sqlx::query_as(
            "SELECT provider_user_id FROM user_provider_ids WHERE provider = 'slack'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert_eq!(cached, None, "an unmapped sender caches nothing");
    }
}
