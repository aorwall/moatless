//! The `preview.*` methods: opening, moving and closing a Task's browser tabs.
//!
//! A tab here is what T3 calls a preview — an in-app browser tab session, not
//! what Moatless calls one. This module stores a URL somebody asked to look at
//! and tells everyone else watching the Task about it; it knows nothing of a
//! preview *server*.
//!
//! The web client cannot see where a tab ended up (cross-origin, opaque), so
//! `navigate` records what was *commanded*; only the desktop client's
//! `<webview>` calls `reportStatus` with what it actually saw.

use std::sync::Arc;

use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::preview_tabs::{PreviewTab, PreviewTabDao, PreviewTabError};
use crate::task_dao::T3TaskDao;
use moatless_types::access_control::ScopeContext;

/// Everything the `preview.*` handlers need.
#[derive(Clone)]
pub struct PreviewState {
    pub tabs: Arc<dyn PreviewTabDao>,
    pub tasks: Arc<dyn T3TaskDao>,
}

/// Why a `preview.*` call could not be answered.
#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    /// The viewer cannot read this Task, or it does not exist.
    #[error("conversation not found")]
    NotFound,
    /// No tab by that id is open on the Task. The contract has a tagged error
    /// for exactly this, so the client renders it as one of the method's
    /// ordinary outcomes rather than as a server defect.
    #[error("unknown preview session")]
    UnknownTab { thread_id: String, tab_id: String },
    #[error("the request did not carry the fields the contract requires")]
    Malformed(&'static str),
    #[error("the preview tab store could not be reached")]
    Store(#[from] PreviewTabError),
    #[error("the task could not be read")]
    Database(#[source] sqlx::Error),
}

/// Render one tab in the shape `PreviewSessionSnapshot` has on the wire.
fn snapshot(tab: &PreviewTab) -> Value {
    let mut value = json!({
        "threadId": tab.thread_id,
        "tabId": tab.tab_id,
        "navStatus": tab.nav_status,
        "canGoBack": tab.can_go_back,
        "canGoForward": tab.can_go_forward,
        "updatedAt": tab.updated_at,
    });
    // Optional in the contract, and absent means fill — so a tab that was
    // never resized sends no key rather than a null the schema would reject.
    if let Some(viewport) = &tab.viewport {
        value["viewport"] = viewport.clone();
    }
    value
}

/// Fail closed, then a scoped read. The same answer for "not yours" and "not
/// there", so a caller cannot probe another tenant's Task ids.
async fn authorize(
    state: &PreviewState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<(), PreviewError> {
    state
        .tasks
        .find_by_id(task_id, viewer)
        .await
        .map_err(PreviewError::Database)?
        .ok_or(PreviewError::NotFound)
        .map(drop)
}

fn thread_id(payload: &Value) -> Result<&str, PreviewError> {
    payload
        .get("threadId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or(PreviewError::Malformed("threadId"))
}

fn tab_id(payload: &Value) -> Result<&str, PreviewError> {
    payload
        .get("tabId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or(PreviewError::Malformed("tabId"))
}

fn url(payload: &Value) -> Result<&str, PreviewError> {
    payload
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or(PreviewError::Malformed("url"))
}

/// A `Success` nav status for a URL nobody has watched load.
///
/// The title is empty deliberately — see the module docs.
fn commanded(url: &str) -> Value {
    json!({ "_tag": "Success", "url": url, "title": "" })
}

/// Store a whole tab, then tell every replica about it.
///
/// The write lands first — a publish failure leaves the tab stored and the
/// caller answered; clients that missed the event converge on their next
/// `preview.list`.
async fn commit(
    state: &PreviewState,
    tab: PreviewTab,
    event_type: &str,
) -> Result<Value, PreviewError> {
    let revision = state.tabs.put(&tab).await?;
    let rendered = snapshot(&tab);
    let event = json!({
        "type": event_type,
        "threadId": tab.thread_id,
        "tabId": tab.tab_id,
        "createdAt": tab.updated_at,
        "serverEpoch": state.tabs.epoch(),
        "revision": revision,
        "snapshot": rendered,
    });
    if let Err(error) = state.tabs.publish(event).await {
        tracing::warn!(%error, tab_id = %tab.tab_id, "A preview event was not published");
    }
    Ok(rendered)
}

pub async fn open(
    state: &PreviewState,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<Value, PreviewError> {
    let task_id = thread_id(payload)?;
    authorize(state, viewer, task_id).await?;
    // `url` is optional here and only here: a tab opened without one is the
    // empty tab a user types into.
    let nav_status = match payload.get("url").and_then(Value::as_str) {
        Some(url) if !url.trim().is_empty() => commanded(url.trim()),
        _ => json!({ "_tag": "Idle" }),
    };
    commit(
        state,
        PreviewTab {
            thread_id: task_id.to_string(),
            // Server-side, which is what lets two clients open a tab at the
            // same moment without coordinating: each gets its own identity.
            tab_id: Uuid::new_v4().to_string(),
            nav_status,
            can_go_back: false,
            can_go_forward: false,
            viewport: None,
            updated_at: Utc::now().to_rfc3339(),
        },
        "opened",
    )
    .await
}

/// Open a tab at a caller-chosen id, replacing whatever that id held.
///
/// Unlike [`open`], which mints one: every run of one Script is the same tab
/// (see [`crate::scripts::run`]), so this re-announces it, carrying the
/// viewport over and pulling every viewer back to `url`.
pub async fn open_at(
    state: &PreviewState,
    viewer: &ScopeContext,
    task_id: &str,
    tab_id: &str,
    url: &str,
) -> Result<Value, PreviewError> {
    authorize(state, viewer, task_id).await?;
    let viewport = state
        .tabs
        .get(task_id, tab_id)
        .await?
        .and_then(|tab| tab.viewport);
    commit(
        state,
        PreviewTab {
            thread_id: task_id.to_string(),
            tab_id: tab_id.to_string(),
            nav_status: commanded(url),
            can_go_back: false,
            can_go_forward: false,
            viewport,
            updated_at: Utc::now().to_rfc3339(),
        },
        "opened",
    )
    .await
}

pub async fn navigate(
    state: &PreviewState,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<Value, PreviewError> {
    let task_id = thread_id(payload)?;
    let tab_id = tab_id(payload)?;
    let url = url(payload)?;
    authorize(state, viewer, task_id).await?;
    let mut tab = require_tab(state, task_id, tab_id).await?;
    tab.nav_status = commanded(url);
    // Whatever the last client reported is no longer true of the page the
    // frame is about to show, and a web client will never correct it.
    tab.can_go_back = false;
    tab.can_go_forward = false;
    tab.updated_at = Utc::now().to_rfc3339();
    commit(state, tab, "navigated").await
}

pub async fn resize(
    state: &PreviewState,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<Value, PreviewError> {
    let task_id = thread_id(payload)?;
    let tab_id = tab_id(payload)?;
    let viewport = payload
        .get("viewport")
        .cloned()
        .ok_or(PreviewError::Malformed("viewport"))?;
    authorize(state, viewer, task_id).await?;
    let mut tab = require_tab(state, task_id, tab_id).await?;
    tab.viewport = Some(viewport);
    tab.updated_at = Utc::now().to_rfc3339();
    commit(state, tab, "resized").await
}

/// Reloading a frame changes nothing anyone else can observe.
///
/// No stored state moves and no event is published — the client that asked
/// re-keys its own frame; publishing one would reload the page for everyone
/// else.
pub async fn refresh(
    state: &PreviewState,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<(), PreviewError> {
    let task_id = thread_id(payload)?;
    let tab_id = tab_id(payload)?;
    authorize(state, viewer, task_id).await?;
    require_tab(state, task_id, tab_id).await.map(drop)
}

/// What the desktop client saw, written verbatim.
///
/// The one method whose values are observations rather than commands — and the
/// only one a web client never calls.
pub async fn report_status(
    state: &PreviewState,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<(), PreviewError> {
    let task_id = thread_id(payload)?;
    let tab_id = tab_id(payload)?;
    let nav_status = payload
        .get("navStatus")
        .cloned()
        .ok_or(PreviewError::Malformed("navStatus"))?;
    authorize(state, viewer, task_id).await?;
    let mut tab = require_tab(state, task_id, tab_id).await?;
    tab.nav_status = nav_status;
    tab.can_go_back = payload
        .get("canGoBack")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    tab.can_go_forward = payload
        .get("canGoForward")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    tab.updated_at = Utc::now().to_rfc3339();
    commit(state, tab, "navigated").await.map(drop)
}

/// Close one tab, or every tab on the Task when no `tabId` is given.
///
/// Closes for everyone who can read the Task — a tab belongs to the Task, not
/// to whoever opened it, matching the desktop app's long-standing behaviour.
pub async fn close(
    state: &PreviewState,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<(), PreviewError> {
    let task_id = thread_id(payload)?;
    let tab_id = payload
        .get("tabId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    authorize(state, viewer, task_id).await?;
    let removal = state.tabs.remove(task_id, tab_id).await?;
    let created_at = Utc::now().to_rfc3339();
    for tab_id in removal.tab_ids {
        // One event per tab, all at the revision the close landed at. The
        // client compares revisions and never counts them, so sharing one
        // across a multi-tab close is not something it can trip over.
        let event = json!({
            "type": "closed",
            "threadId": task_id,
            "tabId": tab_id,
            "createdAt": created_at,
            "serverEpoch": state.tabs.epoch(),
            "revision": removal.revision,
        });
        if let Err(error) = state.tabs.publish(event).await {
            tracing::warn!(%error, tab_id, "A preview close event was not published");
        }
    }
    Ok(())
}

/// Every tab open on a Task, and the position the read was taken at.
///
/// `serverEpoch` and `revision` are load-bearing: the client rejects a listing
/// older than one already applied and refetches wholesale when the epoch moves;
/// without them a listing decodes to nothing and reads as "no tabs".
pub async fn list(
    state: &PreviewState,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<Value, PreviewError> {
    let task_id = thread_id(payload)?;
    authorize(state, viewer, task_id).await?;
    let listing = state.tabs.list(task_id).await?;
    Ok(json!({
        "sessions": listing.tabs.iter().map(snapshot).collect::<Vec<_>>(),
        "serverEpoch": state.tabs.epoch(),
        "revision": listing.revision,
    }))
}

async fn require_tab(
    state: &PreviewState,
    task_id: &str,
    tab_id: &str,
) -> Result<PreviewTab, PreviewError> {
    state
        .tabs
        .get(task_id, tab_id)
        .await?
        .ok_or_else(|| PreviewError::UnknownTab {
            thread_id: task_id.to_string(),
            tab_id: tab_id.to_string(),
        })
}

/// The error a client's schema knows how to decode, when there is one.
///
/// `PreviewSessionLookupError` is declared on every method in the group, so an
/// unknown tab reaches the panel as its own message; everything else renders as
/// an unexpected server failure.
pub fn typed_error(error: &PreviewError) -> Option<Value> {
    match error {
        PreviewError::UnknownTab { thread_id, tab_id } => Some(json!({
            "_tag": "PreviewSessionLookupError",
            "threadId": thread_id,
            "tabId": tab_id,
        })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview_tabs::InMemoryPreviewTabDao;
    use crate::task_dao::{MockT3TaskDao, TaskView};

    fn viewer() -> ScopeContext {
        ScopeContext {
            user_id: "viewer".to_string(),
            is_admin: false,
        }
    }

    /// Inert but for the id: nothing in this module reads a Task's columns,
    /// only whether a scoped read returned one.
    fn task_fixture(_task_id: &str) -> TaskView {
        TaskView {
            status: moatless_types::task::TaskStatus::Open,
            ttl_minutes: 15,
        }
    }

    /// A task DAO that answers for `readable` and for nothing else, which is
    /// how a scoped read behaves for a viewer without access.
    fn tasks_readable(readable: Option<&'static str>) -> Arc<dyn T3TaskDao> {
        let mut tasks = MockT3TaskDao::new();
        tasks.expect_find_by_id().returning(move |task_id, _| {
            Ok((readable == Some(task_id)).then(|| task_fixture(task_id)))
        });
        Arc::new(tasks)
    }

    fn state(readable: Option<&'static str>) -> PreviewState {
        PreviewState {
            tabs: Arc::new(InMemoryPreviewTabDao::new()),
            tasks: tasks_readable(readable),
        }
    }

    /// An empty listing still carries `revision` and `serverEpoch`: the
    /// contract cannot decode a bare `{"sessions": []}`, and the client reads
    /// that failure as "no tabs" rather than as an error.
    #[tokio::test]
    async fn listing_a_task_with_no_tabs_still_carries_an_epoch_and_a_revision() {
        let state = state(Some("task"));
        let listed = list(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        assert_eq!(listed["sessions"], json!([]));
        assert_eq!(listed["revision"], 0);
        assert!(
            listed["serverEpoch"]
                .as_str()
                .is_some_and(|e| !e.is_empty())
        );
    }

    #[tokio::test]
    async fn opening_with_a_url_records_it_as_commanded_with_a_blank_title() {
        let state = state(Some("task"));
        let opened = open(
            &state,
            &viewer(),
            &json!({ "threadId": "task", "url": "https://task--5733.example.com" }),
        )
        .await
        .unwrap();
        assert_eq!(
            opened["navStatus"],
            json!({ "_tag": "Success", "url": "https://task--5733.example.com", "title": "" })
        );
        assert_eq!(opened["canGoBack"], false);
        assert_eq!(opened["threadId"], "task");
        assert!(opened.get("viewport").is_none());
    }

    #[tokio::test]
    async fn opening_without_a_url_is_an_idle_tab() {
        let state = state(Some("task"));
        let opened = open(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        assert_eq!(opened["navStatus"], json!({ "_tag": "Idle" }));
    }

    #[tokio::test]
    async fn an_opened_tab_is_listed_and_a_closed_one_is_not() {
        let state = state(Some("task"));
        let opened = open(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        let tab_id = opened["tabId"].as_str().unwrap().to_string();
        let listed = list(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        assert_eq!(listed["sessions"].as_array().unwrap().len(), 1);

        close(
            &state,
            &viewer(),
            &json!({ "threadId": "task", "tabId": tab_id }),
        )
        .await
        .unwrap();
        let listed = list(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        assert_eq!(listed["sessions"], json!([]));
    }

    #[tokio::test]
    async fn navigating_moves_the_url_and_publishes_an_event() {
        let state = state(Some("task"));
        let mut events = state.tabs.subscribe();
        let opened = open(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        assert_eq!(events.recv().await.unwrap()["type"], "opened");

        let tab_id = opened["tabId"].as_str().unwrap();
        let moved = navigate(
            &state,
            &viewer(),
            &json!({ "threadId": "task", "tabId": tab_id, "url": "https://elsewhere.example" }),
        )
        .await
        .unwrap();
        assert_eq!(moved["navStatus"]["url"], "https://elsewhere.example");
        let event = events.recv().await.unwrap();
        assert_eq!(event["type"], "navigated");
        assert_eq!(
            event["snapshot"]["navStatus"]["url"],
            "https://elsewhere.example"
        );
        assert!(event["revision"].as_u64().unwrap() > 0);
    }

    /// Refresh is the one method with no observable effect. If it ever starts
    /// publishing, every other client reloads a page nobody asked them to.
    #[tokio::test]
    async fn refreshing_publishes_nothing_and_moves_no_revision() {
        let state = state(Some("task"));
        let opened = open(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        let before = list(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        let mut events = state.tabs.subscribe();

        refresh(
            &state,
            &viewer(),
            &json!({ "threadId": "task", "tabId": opened["tabId"] }),
        )
        .await
        .unwrap();

        let after = list(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        assert_eq!(before["revision"], after["revision"]);
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn resizing_stores_the_viewport_verbatim() {
        let state = state(Some("task"));
        let opened = open(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        let viewport =
            json!({ "_tag": "preset", "width": 390, "height": 844, "presetId": "iphone-12-pro" });
        let resized = resize(
            &state,
            &viewer(),
            &json!({ "threadId": "task", "tabId": opened["tabId"], "viewport": viewport }),
        )
        .await
        .unwrap();
        assert_eq!(resized["viewport"], viewport);
    }

    #[tokio::test]
    async fn the_desktop_client_s_report_is_written_verbatim() {
        let state = state(Some("task"));
        let opened = open(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        let nav_status = json!({
            "_tag": "LoadFailed", "url": "https://a.example", "title": "t",
            "code": -105, "description": "NAME_NOT_RESOLVED"
        });
        report_status(
            &state,
            &viewer(),
            &json!({
                "threadId": "task", "tabId": opened["tabId"], "navStatus": nav_status,
                "canGoBack": true, "canGoForward": false
            }),
        )
        .await
        .unwrap();
        let listed = list(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        assert_eq!(listed["sessions"][0]["navStatus"], nav_status);
        assert_eq!(listed["sessions"][0]["canGoBack"], true);
    }

    #[tokio::test]
    async fn closing_without_a_tab_id_closes_every_tab_on_the_task() {
        let state = state(Some("task"));
        open(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        open(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        close(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        let listed = list(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        assert_eq!(listed["sessions"], json!([]));
    }

    #[tokio::test]
    async fn an_unknown_tab_is_a_typed_error_the_client_can_decode() {
        let state = state(Some("task"));
        let error = navigate(
            &state,
            &viewer(),
            &json!({ "threadId": "task", "tabId": "nope", "url": "https://a.example" }),
        )
        .await
        .unwrap_err();
        assert_eq!(
            typed_error(&error),
            Some(json!({
                "_tag": "PreviewSessionLookupError",
                "threadId": "task",
                "tabId": "nope"
            }))
        );
    }

    /// The reason `open_at` exists: running the same Script again must land in
    /// the tab that Script already has, or a morning of restarts leaves a tab
    /// strip of dead duplicates.
    #[tokio::test]
    async fn opening_at_the_same_id_twice_leaves_one_tab() {
        let state = state(Some("task"));
        let mut events = state.tabs.subscribe();

        for url in [
            "https://task--5733.example.com",
            "https://task--5733.example.com/next",
        ] {
            open_at(&state, &viewer(), "task", "script-dev", url)
                .await
                .unwrap();
        }

        let listed = list(&state, &viewer(), &json!({ "threadId": "task" }))
            .await
            .unwrap();
        assert_eq!(listed["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(listed["sessions"][0]["tabId"], "script-dev");
        assert_eq!(
            listed["sessions"][0]["navStatus"]["url"],
            "https://task--5733.example.com/next"
        );
        // Both runs announce themselves: the second is how a client that has
        // the tab already is told to show it again.
        for _ in 0..2 {
            assert_eq!(events.recv().await.unwrap()["type"], "opened");
        }
    }

    /// A re-run is the same tab, so a size somebody chose has to survive it —
    /// otherwise every restart of a Script snaps the panel back to fill.
    #[tokio::test]
    async fn re_opening_at_an_id_keeps_the_viewport_the_tab_was_resized_to() {
        let state = state(Some("task"));
        let viewport =
            json!({ "_tag": "preset", "width": 390, "height": 844, "presetId": "iphone-12-pro" });
        open_at(&state, &viewer(), "task", "script-dev", "https://a.example")
            .await
            .unwrap();
        resize(
            &state,
            &viewer(),
            &json!({ "threadId": "task", "tabId": "script-dev", "viewport": viewport }),
        )
        .await
        .unwrap();

        let reopened = open_at(&state, &viewer(), "task", "script-dev", "https://a.example")
            .await
            .unwrap();

        assert_eq!(reopened["viewport"], viewport);
    }

    /// Every method in the group authorizes first, so a viewer without access
    /// cannot open, list, navigate or close a tab on someone else's Task.
    #[tokio::test]
    async fn a_viewer_without_access_is_refused_by_every_method() {
        let state = state(None);
        let payload = json!({ "threadId": "task", "tabId": "a", "url": "https://a.example",
                              "viewport": { "_tag": "fill" }, "navStatus": { "_tag": "Idle" } });
        assert!(matches!(
            open(&state, &viewer(), &payload).await,
            Err(PreviewError::NotFound)
        ));
        assert!(matches!(
            open_at(&state, &viewer(), "task", "a", "https://a.example").await,
            Err(PreviewError::NotFound)
        ));
        assert!(matches!(
            list(&state, &viewer(), &payload).await,
            Err(PreviewError::NotFound)
        ));
        assert!(matches!(
            navigate(&state, &viewer(), &payload).await,
            Err(PreviewError::NotFound)
        ));
        assert!(matches!(
            resize(&state, &viewer(), &payload).await,
            Err(PreviewError::NotFound)
        ));
        assert!(matches!(
            refresh(&state, &viewer(), &payload).await,
            Err(PreviewError::NotFound)
        ));
        assert!(matches!(
            report_status(&state, &viewer(), &payload).await,
            Err(PreviewError::NotFound)
        ));
        assert!(matches!(
            close(&state, &viewer(), &payload).await,
            Err(PreviewError::NotFound)
        ));
    }
}
