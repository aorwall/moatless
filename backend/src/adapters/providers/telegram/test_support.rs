//! A local Bot API stub that records what the Adapter sent it, and the one
//! Subscription fixture its tests build on.
//!
//! [`crate::adapters::test_support::stub_provider`] answers, but remembers
//! nothing — and half of what the Telegram workflows promise is a *count*: one
//! `sendMessage` per reply, one per `/start`, one per pairing code, and none at
//! all from normalization. A stub that cannot be counted cannot prove that.

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::any;
use axum::{Json, Router};
use moatless_backend_core::scope::Scope;

use crate::adapters::models::{LoopContext, RoutingMode, SubscriptionMatch};

/// The Subscription's run-as user — the fallback identity an unmapped Telegram
/// sender is routed as.
pub(super) const OWNER: &str = "user-owner";

/// An active Telegram Subscription matching one chat.
///
/// `SubscriptionMatch` has twenty-odd fields and every module's tests need two
/// of them; spelling the rest out per module is how two copies drift into
/// disagreeing about what a default Subscription is. Set what your test is
/// about on the value this returns.
pub(super) fn test_rule(provider: &str, source_matcher: &str) -> SubscriptionMatch {
    let now = chrono::Utc::now();
    SubscriptionMatch {
        id: "tt_test".into(),
        provider: provider.into(),
        connection_id: "pc_test".into(),
        source_matcher: source_matcher.into(),
        source_name: None,
        routing_mode: RoutingMode::BySubject,
        event_filter: serde_json::json!([]),
        repository_id: "repo1".into(),
        prompt_template: None,
        task_name: None,
        tag_ids: vec![],
        run_as_user_id: None,
        agent_type: "claude-code".into(),
        inactive_ttl_days: None,
        allow_direct_send: true,
        provider_config: None,
        ongoing_task_id: None,
        close_task_on_close: true,
        notify_on_success: false,
        active: true,
        created_by: "system".into(),
        scope: Scope::Global,
        owner_user_id: None,
        created_at: now,
        updated_at: now,
    }
}

/// One Bot API call the Adapter made, as the stub saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordedCall {
    /// The Bot API method, e.g. `sendMessage`.
    pub(super) method: String,
    /// The JSON body, or `null` for a bodiless call.
    pub(super) body: serde_json::Value,
}

/// A running Bot API stub. Hand [`Self::base_url`] to
/// [`super::api::TelegramApi::with_base_url`].
pub(super) struct BotApiStub {
    base_url: String,
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

impl BotApiStub {
    pub(super) fn base_url(&self) -> String {
        self.base_url.clone()
    }

    /// Every call the Adapter made, in order.
    pub(super) fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().expect("stub call log").clone()
    }

    /// The names of the Bot API methods called, in order — the API-call budget
    /// most assertions are about.
    pub(super) fn methods(&self) -> Vec<String> {
        self.calls().into_iter().map(|call| call.method).collect()
    }
}

/// The same Subscription as [`test_rule`], in the shape the workflow paths see
/// it: one chat, no provider config, both access policies at their defaults.
pub(super) fn test_loop_context(source_id: &str) -> LoopContext {
    let now = chrono::Utc::now();
    LoopContext {
        id: "tt_test".into(),
        provider: "telegram".into(),
        source_id: source_id.into(),
        source_name: None,
        repository_id: "repo-1".into(),
        routing_mode: RoutingMode::BySubject,
        prompt_template: None,
        active: true,
        allow_direct_send: false,
        provider_config: None,
        ongoing_task_id: None,
        scope: Scope::User,
        task_name: None,
        tag_ids: vec![],
        run_as_user_id: Some(OWNER.into()),
        agent_type: "claude-code".into(),
        inactive_ttl_days: None,
        created_by: "system".into(),
        created_at: now,
        updated_at: now,
    }
}

/// Serve `result` as the `ok: true` payload of every Bot API method, recording
/// each call. The server lives as long as the test process.
pub(super) async fn bot_api_stub(result: serde_json::Value) -> BotApiStub {
    #[derive(Clone)]
    struct StubState {
        calls: Arc<Mutex<Vec<RecordedCall>>>,
        result: serde_json::Value,
    }

    async fn handle(
        State(state): State<StubState>,
        request: axum::extract::Request,
    ) -> Json<serde_json::Value> {
        let method = request
            .uri()
            .path()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string();
        let body = axum::body::to_bytes(request.into_body(), usize::MAX)
            .await
            .expect("read stub request body");
        let body = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        state
            .calls
            .lock()
            .expect("stub call log")
            .push(RecordedCall { method, body });

        Json(serde_json::json!({ "ok": true, "result": state.result }))
    }

    let calls = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let app = Router::new().fallback(any(handle)).with_state(StubState {
        calls: Arc::clone(&calls),
        result,
    });
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    BotApiStub {
        base_url: format!("http://{addr}"),
        calls,
    }
}
