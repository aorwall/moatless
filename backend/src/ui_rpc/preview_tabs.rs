//! Where the browser tabs open on a Task live.
//!
//! A tab is a fact about a Task rather than about a person or a device: two
//! reviewers looking at the same conversation see the same tabs, and closing
//! one closes it for both. That is the whole reason this is a store at all
//! rather than client state — see `.plans/preview-servers-in-t3-web.md`,
//! Decision 3.
//!
//! Two implementations, and neither is a test double. The backend runs two
//! replicas in Kubernetes (`k8s/helm/moatless-vibe/values.yaml`), so an
//! in-process map would make "which tabs exist" depend on which replica a
//! socket landed on; Redis is the production answer. But Redis is optional
//! configuration (`startup::init_event_publisher`), and a Docker or local
//! deployment without it runs exactly one process — where the in-memory
//! implementation is not a compromise but the correct one.
//!
//! What is stored is a whole snapshot per tab. Every mutation writes the
//! complete value and bumps a revision, so there is no read-modify-write to
//! lose a race on and no partial write to compensate for: revisions are
//! compared, never counted.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

/// Redis channel every replica publishes rendered `PreviewEvent`s to.
///
/// Pub/sub rather than a stream, deliberately: a client that missed events
/// recovers by re-reading `preview.list` under the epoch it holds, so there is
/// nothing a replay would add and a retained backlog would only be read once.
const EVENT_CHANNEL: &str = "preview:events";

/// The key holding this store's epoch, written once with `SET NX`.
const EPOCH_KEY: &str = "preview:epoch";

/// Events buffered per connected subscriber before the slowest one starts
/// losing them.
///
/// Overflow is survivable and needs no handling: a subscriber that lagged
/// receives `RecvError::Lagged`, and the tab list it is holding is reconciled
/// against `preview.list` on the next mutation it does see. Deep enough that a
/// burst of opens on one Task cannot cause that in practice.
const EVENT_BUFFER: usize = 256;

/// How long a Task's tabs outlive their last mutation.
///
/// Passed in rather than chosen here so it can track the sandbox idle TTL — a
/// tab should not outlive the servers it points at. Refreshed on every write.
#[derive(Debug, Clone, Copy)]
pub struct PreviewTabTtl(pub Duration);

/// One browser tab, in the shape `PreviewSessionSnapshot` has on the wire
/// (`packages/contracts/src/preview.ts`).
///
/// `nav_status` and `viewport` are carried as JSON rather than re-modelled:
/// both are tagged unions this backend never inspects, only stores and returns.
/// Transcribing them into Rust enums would add a second place for the contract
/// to drift without giving any code here a decision to make.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewTab {
    pub thread_id: String,
    pub tab_id: String,
    pub nav_status: Value,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewport: Option<Value>,
    pub updated_at: String,
}

/// Every tab on one Task, and the position that read was taken at.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewTabListing {
    pub tabs: Vec<PreviewTab>,
    pub revision: u64,
}

/// What a close removed, and the revision it removed them at.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewTabRemoval {
    pub tab_ids: Vec<String>,
    pub revision: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum PreviewTabError {
    /// The store could not be reached or refused the command. Every operation
    /// here is safe to retry, because every write is a whole snapshot.
    #[error("the preview tab store is unavailable")]
    Unavailable(#[source] redis::RedisError),
    /// A stored snapshot could not be read back. Retrying cannot help; the
    /// value is skipped and the rest of the listing is still answered.
    #[error("a stored preview tab could not be decoded")]
    Undecodable(#[source] serde_json::Error),
}

/// The set of tabs open on each Task, and the fan-out that tells every replica
/// when one changes.
#[async_trait]
pub trait PreviewTabDao: Send + Sync {
    /// Identifies the store's *contents*, not the process reading them. A
    /// client compares it to decide whether the revisions it holds are
    /// comparable at all, so it must survive a restart of this handle and must
    /// change when the store is emptied.
    fn epoch(&self) -> &str;

    async fn list(&self, task_id: &str) -> Result<PreviewTabListing, PreviewTabError>;

    async fn get(&self, task_id: &str, tab_id: &str)
        -> Result<Option<PreviewTab>, PreviewTabError>;

    /// Write one whole tab and return the revision the write landed at.
    async fn put(&self, tab: &PreviewTab) -> Result<u64, PreviewTabError>;

    /// Remove one tab, or every tab on the Task when `tab_id` is `None`.
    async fn remove(
        &self,
        task_id: &str,
        tab_id: Option<&str>,
    ) -> Result<PreviewTabRemoval, PreviewTabError>;

    /// Fan one rendered `PreviewEvent` to every replica, including this one.
    async fn publish(&self, event: Value) -> Result<(), PreviewTabError>;

    /// Every event published from now on, by any replica.
    fn subscribe(&self) -> broadcast::Receiver<Value>;
}

fn tabs_key(task_id: &str) -> String {
    format!("preview:{task_id}")
}

fn revision_key(task_id: &str) -> String {
    format!("preview:{task_id}:rev")
}

/// Decode a hash of `tab_id -> JSON` into tabs, skipping what cannot be read.
///
/// A single undecodable field is not a reason to fail the whole listing: the
/// caller asked what tabs are open, and the honest answer is the ones that
/// could be read. The skipped field is logged once.
fn decode_tabs(fields: HashMap<String, String>) -> Vec<PreviewTab> {
    let mut tabs: Vec<PreviewTab> = fields
        .into_iter()
        .filter_map(|(tab_id, encoded)| match serde_json::from_str(&encoded) {
            Ok(tab) => Some(tab),
            Err(error) => {
                warn!(%error, tab_id, "Skipping an undecodable stored preview tab");
                None
            }
        })
        .collect();
    // A stable order so two reads of an unchanged store are the same answer.
    tabs.sort_by(|left, right| left.tab_id.cmp(&right.tab_id));
    tabs
}

// =============================================================================
// Redis
// =============================================================================

/// The production store: tabs in a Redis hash per Task, events over pub/sub.
pub struct RedisPreviewTabDao {
    conn: redis::aio::ConnectionManager,
    epoch: String,
    ttl_seconds: u64,
    events: broadcast::Sender<Value>,
}

impl RedisPreviewTabDao {
    /// Connect, settle the epoch, and start forwarding published events.
    ///
    /// The returned handle is the subscriber task. It is the caller's to
    /// supervise: while it is not running this replica still serves every read
    /// and write, but its connected clients stop hearing about tabs opened on
    /// the other replica.
    pub async fn connect(
        redis_url: &str,
        ttl: PreviewTabTtl,
    ) -> Result<(Self, JoinHandle<()>), redis::RedisError> {
        let client = redis::Client::open(redis_url)?;
        let mut conn = redis::aio::ConnectionManager::new_with_config(
            client.clone(),
            crate::events::redis_manager_config(),
        )
        .await?;

        // `SET NX` then `GET`: the first process to reach a fresh Redis names
        // the epoch and every later one adopts it, so a restart does not
        // invalidate revisions the clients still hold. Flushing Redis is what
        // changes it, which is exactly the event the epoch exists to signal.
        let _: bool = conn.set_nx(EPOCH_KEY, Uuid::new_v4().to_string()).await?;
        let epoch: String = conn.get(EPOCH_KEY).await?;

        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let forwarder = tokio::spawn(forward_published_events(client, events.clone()));
        info!(%epoch, "Preview tab store connected to Redis");
        Ok((
            Self {
                conn,
                epoch,
                ttl_seconds: ttl.0.as_secs().max(1),
                events,
            },
            forwarder,
        ))
    }
}

/// Re-publish everything on the Redis channel onto the local broadcast.
///
/// Runs until aborted. A dropped subscription is reconnected rather than
/// surfaced, because the only thing a caller could do with the error is
/// reconnect: the loop is the error handling. Each reconnect gap costs
/// connected clients the events published during it, and they recover on the
/// next mutation they do see.
async fn forward_published_events(client: redis::Client, events: broadcast::Sender<Value>) {
    /// Long enough that a Redis outage does not become a reconnect storm,
    /// short enough that a tab opened on the other replica appears promptly
    /// once it is back.
    const RECONNECT_DELAY: Duration = Duration::from_secs(2);

    loop {
        match client.get_async_pubsub().await {
            Ok(mut pubsub) => {
                if let Err(error) = pubsub.subscribe(EVENT_CHANNEL).await {
                    warn!(%error, "Could not subscribe to the preview event channel");
                } else {
                    let mut stream = pubsub.on_message();
                    while let Some(message) = stream.next().await {
                        let Ok(payload) = message.get_payload::<String>() else {
                            continue;
                        };
                        match serde_json::from_str::<Value>(&payload) {
                            // A send with no receivers is the ordinary case —
                            // this replica may have no sockets open.
                            Ok(event) => drop(events.send(event)),
                            Err(error) => {
                                warn!(%error, "Discarding an undecodable published preview event");
                            }
                        }
                    }
                    warn!("The preview event subscription ended; reconnecting");
                }
            }
            Err(error) => warn!(%error, "Could not reach Redis for preview events"),
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

#[async_trait]
impl PreviewTabDao for RedisPreviewTabDao {
    fn epoch(&self) -> &str {
        &self.epoch
    }

    async fn list(&self, task_id: &str) -> Result<PreviewTabListing, PreviewTabError> {
        let mut conn = self.conn.clone();
        let (fields, revision): (HashMap<String, String>, Option<u64>) = redis::pipe()
            .hgetall(tabs_key(task_id))
            .get(revision_key(task_id))
            .query_async(&mut conn)
            .await
            .map_err(PreviewTabError::Unavailable)?;
        Ok(PreviewTabListing {
            tabs: decode_tabs(fields),
            revision: revision.unwrap_or(0),
        })
    }

    async fn get(
        &self,
        task_id: &str,
        tab_id: &str,
    ) -> Result<Option<PreviewTab>, PreviewTabError> {
        let mut conn = self.conn.clone();
        let encoded: Option<String> = conn
            .hget(tabs_key(task_id), tab_id)
            .await
            .map_err(PreviewTabError::Unavailable)?;
        encoded
            .map(|encoded| serde_json::from_str(&encoded).map_err(PreviewTabError::Undecodable))
            .transpose()
    }

    async fn put(&self, tab: &PreviewTab) -> Result<u64, PreviewTabError> {
        let encoded = serde_json::to_string(tab).map_err(PreviewTabError::Undecodable)?;
        let tabs = tabs_key(&tab.thread_id);
        let revision = revision_key(&tab.thread_id);
        let mut conn = self.conn.clone();
        // One round trip. If the INCR lands and the HSET does not, the revision
        // has advanced past a state that was never stored — harmless, because
        // revisions are compared and never counted.
        let ((), next, (), ()): ((), u64, (), ()) = redis::pipe()
            .hset(&tabs, &tab.tab_id, encoded)
            .incr(&revision, 1)
            .expire(&tabs, self.ttl_seconds as i64)
            .expire(&revision, self.ttl_seconds as i64)
            .query_async(&mut conn)
            .await
            .map_err(PreviewTabError::Unavailable)?;
        Ok(next)
    }

    async fn remove(
        &self,
        task_id: &str,
        tab_id: Option<&str>,
    ) -> Result<PreviewTabRemoval, PreviewTabError> {
        let tabs = tabs_key(task_id);
        let revision = revision_key(task_id);
        let mut conn = self.conn.clone();
        let removed: Vec<String> = if let Some(tab_id) = tab_id {
            let existed: u64 = conn
                .hdel(&tabs, tab_id)
                .await
                .map_err(PreviewTabError::Unavailable)?;
            if existed == 0 {
                Vec::new()
            } else {
                vec![tab_id.to_string()]
            }
        } else {
            let mut keys: Vec<String> = conn
                .hkeys(&tabs)
                .await
                .map_err(PreviewTabError::Unavailable)?;
            if !keys.is_empty() {
                let _: u64 = conn.del(&tabs).await.map_err(PreviewTabError::Unavailable)?;
            }
            // Hash fields come back in whatever order Redis holds them. Sorting
            // is the same rule `decode_tabs` follows and for the same reason:
            // the two implementations have to be one answer, and a client
            // rendering a close event should not see the order change.
            keys.sort();
            keys
        };
        // Bump even when nothing was removed: a close of an already-closed tab
        // still answers success, and a revision that moved without a change is
        // one a client accepts and re-reads at.
        let (next, (), ()): (u64, (), ()) = redis::pipe()
            .incr(&revision, 1)
            .expire(&tabs, self.ttl_seconds as i64)
            .expire(&revision, self.ttl_seconds as i64)
            .query_async(&mut conn)
            .await
            .map_err(PreviewTabError::Unavailable)?;
        Ok(PreviewTabRemoval {
            tab_ids: removed,
            revision: next,
        })
    }

    async fn publish(&self, event: Value) -> Result<(), PreviewTabError> {
        let payload = serde_json::to_string(&event).map_err(PreviewTabError::Undecodable)?;
        let mut conn = self.conn.clone();
        let _: u64 = conn
            .publish(EVENT_CHANNEL, payload)
            .await
            .map_err(PreviewTabError::Unavailable)?;
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }
}

// =============================================================================
// In memory
// =============================================================================

/// The single-process store, for a deployment that configures no Redis.
///
/// It agrees with the Redis one about everything a caller can observe, which
/// is what lets the same test body run against both. What it cannot do is
/// agree with a *second* process, which is why it is not the production
/// implementation.
pub struct InMemoryPreviewTabDao {
    epoch: String,
    state: Mutex<HashMap<String, TaskTabs>>,
    events: broadcast::Sender<Value>,
}

#[derive(Debug, Default)]
struct TaskTabs {
    tabs: HashMap<String, PreviewTab>,
    revision: u64,
}

impl InMemoryPreviewTabDao {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        Self {
            // A fresh epoch per process is correct here and not a shortcut:
            // the contents die with the process, so a restart genuinely is a
            // different store and clients should discard what they hold.
            epoch: Uuid::new_v4().to_string(),
            state: Mutex::new(HashMap::new()),
            events,
        }
    }
}

impl InMemoryPreviewTabDao {
    /// The guard never crosses an `.await`, so the only way this lock is
    /// poisoned is a panic inside one of the short critical sections below.
    /// The map is still coherent after one — every mutation is a single
    /// insert, remove or drain — so recovering beats propagating the panic to
    /// every later caller.
    fn locked(&self) -> std::sync::MutexGuard<'_, HashMap<String, TaskTabs>> {
        self.state.lock().unwrap_or_else(|poisoned| {
            warn!("The in-process preview tab lock was poisoned; continuing");
            poisoned.into_inner()
        })
    }
}

impl Default for InMemoryPreviewTabDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PreviewTabDao for InMemoryPreviewTabDao {
    fn epoch(&self) -> &str {
        &self.epoch
    }

    async fn list(&self, task_id: &str) -> Result<PreviewTabListing, PreviewTabError> {
        let state = self.locked();
        let Some(task) = state.get(task_id) else {
            return Ok(PreviewTabListing {
                tabs: Vec::new(),
                revision: 0,
            });
        };
        let mut tabs: Vec<PreviewTab> = task.tabs.values().cloned().collect();
        tabs.sort_by(|left, right| left.tab_id.cmp(&right.tab_id));
        Ok(PreviewTabListing {
            tabs,
            revision: task.revision,
        })
    }

    async fn get(
        &self,
        task_id: &str,
        tab_id: &str,
    ) -> Result<Option<PreviewTab>, PreviewTabError> {
        let state = self.locked();
        Ok(state
            .get(task_id)
            .and_then(|task| task.tabs.get(tab_id))
            .cloned())
    }

    async fn put(&self, tab: &PreviewTab) -> Result<u64, PreviewTabError> {
        let mut state = self.locked();
        let task = state.entry(tab.thread_id.clone()).or_default();
        task.tabs.insert(tab.tab_id.clone(), tab.clone());
        task.revision += 1;
        Ok(task.revision)
    }

    async fn remove(
        &self,
        task_id: &str,
        tab_id: Option<&str>,
    ) -> Result<PreviewTabRemoval, PreviewTabError> {
        let mut state = self.locked();
        let task = state.entry(task_id.to_string()).or_default();
        let mut tab_ids: Vec<String> = match tab_id {
            Some(tab_id) => task
                .tabs
                .remove(tab_id)
                .map(|tab| vec![tab.tab_id])
                .unwrap_or_default(),
            None => task.tabs.drain().map(|(tab_id, _)| tab_id).collect(),
        };
        tab_ids.sort();
        task.revision += 1;
        Ok(PreviewTabRemoval {
            tab_ids,
            revision: task.revision,
        })
    }

    async fn publish(&self, event: Value) -> Result<(), PreviewTabError> {
        drop(self.events.send(event));
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }
}

/// Build the store this deployment's configuration calls for.
///
/// Returns the subscriber task alongside it when there is one to supervise;
/// the in-memory store publishes straight onto its own broadcast and has none.
pub async fn build(
    redis_url: Option<&str>,
    ttl: PreviewTabTtl,
) -> (Arc<dyn PreviewTabDao>, Option<JoinHandle<()>>) {
    let Some(redis_url) = redis_url else {
        info!("REDIS_URL not set — preview tabs are held in this process only");
        return (Arc::new(InMemoryPreviewTabDao::new()), None);
    };
    match RedisPreviewTabDao::connect(redis_url, ttl).await {
        Ok((store, forwarder)) => (Arc::new(store), Some(forwarder)),
        Err(error) => {
            // Degrading to the in-process store beats refusing to start: every
            // other T3 surface works without tabs, and a single-replica
            // deployment is indistinguishable from the Redis-backed one.
            warn!(%error, "Could not reach Redis for preview tabs — holding them in this process");
            (Arc::new(InMemoryPreviewTabDao::new()), None)
        }
    }
}

/// What both implementations have to agree about.
///
/// Neither of them is a test double, so "does the in-memory one behave like
/// the Redis one" is a question about two production paths and not about a
/// mock. These cases are the answer, and they are `pub` so the integration
/// tier — the only one with a Redis to point them at — runs the same list.
#[cfg(any(test, feature = "db-tests"))]
// reason: assertions. A case that cannot reach the store has nothing to report
// and should fail loudly; `db-tests` compiles this into the lib, where the
// `#[cfg(test)]` relaxation these lints normally get does not apply.
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod conformance {
    use super::*;
    use serde_json::json;

    /// The two Task ids every case below works under.
    ///
    /// Fixed rather than generated, so a caller pointing these at a store
    /// shared with anything else — which the integration tier does, at a real
    /// Redis — owns them for the length of the run.
    pub const TASK: &str = "task";
    pub const OTHER: &str = "other";

    pub fn tab(task_id: &str, tab_id: &str, url: &str) -> PreviewTab {
        PreviewTab {
            thread_id: task_id.to_string(),
            tab_id: tab_id.to_string(),
            nav_status: json!({ "_tag": "Success", "url": url, "title": "" }),
            can_go_back: false,
            can_go_forward: false,
            viewport: None,
            updated_at: "2026-07-31T00:00:00Z".to_string(),
        }
    }

    /// Start from no tabs.
    ///
    /// Each case says what it starts from rather than assuming, because
    /// `run_every_case` runs them all against one handle and a store that
    /// persists — the property the Redis one exists for — would otherwise
    /// carry each case's tabs into the next.
    async fn reset(store: &dyn PreviewTabDao) {
        store.remove(TASK, None).await.unwrap();
        store.remove(OTHER, None).await.unwrap();
    }

    pub async fn opening_a_tab_lists_it(store: &dyn PreviewTabDao) {
        reset(store).await;
        assert_eq!(store.list(TASK).await.unwrap().tabs, Vec::new());
        let revision = store.put(&tab(TASK, "a", "https://a")).await.unwrap();
        let listed = store.list(TASK).await.unwrap();
        assert_eq!(listed.revision, revision);
        assert_eq!(listed.tabs, vec![tab(TASK, "a", "https://a")]);
    }

    pub async fn revisions_only_move_forward(store: &dyn PreviewTabDao) {
        reset(store).await;
        let first = store.put(&tab(TASK, "a", "https://a")).await.unwrap();
        let second = store.put(&tab(TASK, "b", "https://b")).await.unwrap();
        let third = store.put(&tab(TASK, "a", "https://c")).await.unwrap();
        let closed = store.remove(TASK, Some("a")).await.unwrap().revision;
        assert!(first < second && second < third && third < closed);
    }

    pub async fn navigating_replaces_the_whole_tab(store: &dyn PreviewTabDao) {
        reset(store).await;
        store.put(&tab(TASK, "a", "https://a")).await.unwrap();
        store.put(&tab(TASK, "a", "https://b")).await.unwrap();
        let listed = store.list(TASK).await.unwrap();
        assert_eq!(listed.tabs, vec![tab(TASK, "a", "https://b")]);
    }

    pub async fn closing_without_a_tab_id_closes_every_tab(store: &dyn PreviewTabDao) {
        reset(store).await;
        store.put(&tab(TASK, "a", "https://a")).await.unwrap();
        store.put(&tab(TASK, "b", "https://b")).await.unwrap();
        store.put(&tab(OTHER, "c", "https://c")).await.unwrap();
        let removal = store.remove(TASK, None).await.unwrap();
        assert_eq!(removal.tab_ids, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(store.list(TASK).await.unwrap().tabs, Vec::new());
        // Closing one Task's tabs must not reach another's.
        assert_eq!(store.list(OTHER).await.unwrap().tabs.len(), 1);
    }

    pub async fn closing_an_unknown_tab_removes_nothing(store: &dyn PreviewTabDao) {
        reset(store).await;
        store.put(&tab(TASK, "a", "https://a")).await.unwrap();
        let removal = store.remove(TASK, Some("missing")).await.unwrap();
        assert_eq!(removal.tab_ids, Vec::<String>::new());
        assert_eq!(store.list(TASK).await.unwrap().tabs.len(), 1);
    }

    pub async fn a_published_event_reaches_a_subscriber(store: &dyn PreviewTabDao) {
        let mut events = store.subscribe();
        let published = json!({ "type": "opened" });
        // Two things make a single publish-then-receive the wrong assertion
        // against Redis. Its pub/sub reaches whoever is subscribed at that
        // instant and nobody else, and a handle's forwarder subscribes
        // asynchronously after `connect` has already returned — so an early
        // publish is simply dropped. And the channel is shared with whatever
        // else points at that database, so the next event to arrive is not
        // necessarily this one. Republishing until this event lands asserts
        // that the channel reaches this subscriber, and nothing about when.
        let delivered = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                store.publish(published.clone()).await.unwrap();
                let deadline = tokio::time::Instant::now() + Duration::from_millis(250);
                while let Ok(Ok(event)) = tokio::time::timeout_at(deadline, events.recv()).await {
                    if event == published {
                        return;
                    }
                }
            }
        })
        .await;
        assert!(
            delivered.is_ok(),
            "a published event should reach a subscriber on the same handle"
        );
    }

    /// Every case above, in one call, for a caller that only wants "does this
    /// handle behave". Each case resets first, so they compose.
    pub async fn run_every_case(store: &dyn PreviewTabDao) {
        opening_a_tab_lists_it(store).await;
        revisions_only_move_forward(store).await;
        navigating_replaces_the_whole_tab(store).await;
        closing_without_a_tab_id_closes_every_tab(store).await;
        closing_an_unknown_tab_removes_nothing(store).await;
        a_published_event_reaches_a_subscriber(store).await;
        reset(store).await;
    }
}

#[cfg(test)]
mod tests {
    use super::conformance::tab;
    use super::*;
    use serde_json::json;

    /// Each case gets its own handle, so one failing does not leave state for
    /// the next and a failure names the behaviour that broke.
    #[tokio::test]
    async fn opening_a_tab_lists_it() {
        conformance::opening_a_tab_lists_it(&InMemoryPreviewTabDao::new()).await;
    }

    #[tokio::test]
    async fn revisions_only_move_forward() {
        conformance::revisions_only_move_forward(&InMemoryPreviewTabDao::new()).await;
    }

    #[tokio::test]
    async fn navigating_replaces_the_whole_tab() {
        conformance::navigating_replaces_the_whole_tab(&InMemoryPreviewTabDao::new()).await;
    }

    #[tokio::test]
    async fn closing_without_a_tab_id_closes_every_tab() {
        conformance::closing_without_a_tab_id_closes_every_tab(&InMemoryPreviewTabDao::new()).await;
    }

    #[tokio::test]
    async fn closing_an_unknown_tab_removes_nothing() {
        conformance::closing_an_unknown_tab_removes_nothing(&InMemoryPreviewTabDao::new()).await;
    }

    #[tokio::test]
    async fn a_published_event_reaches_a_subscriber() {
        conformance::a_published_event_reaches_a_subscriber(&InMemoryPreviewTabDao::new()).await;
    }

    #[tokio::test]
    async fn the_epoch_is_stable_for_the_life_of_a_handle() {
        let store = InMemoryPreviewTabDao::new();
        assert_eq!(store.epoch(), store.epoch());
        assert!(!store.epoch().is_empty());
        assert_ne!(store.epoch(), InMemoryPreviewTabDao::new().epoch());
    }

    #[test]
    fn a_stored_tab_round_trips_through_the_wire_shape() {
        let encoded = serde_json::to_value(tab("task", "a", "https://a")).unwrap();
        assert_eq!(
            encoded,
            json!({
                "threadId": "task",
                "tabId": "a",
                "navStatus": { "_tag": "Success", "url": "https://a", "title": "" },
                "canGoBack": false,
                "canGoForward": false,
                "updatedAt": "2026-07-31T00:00:00Z"
            })
        );
    }

    #[test]
    fn an_undecodable_field_is_skipped_rather_than_failing_the_listing() {
        let fields = HashMap::from([
            (
                "a".to_string(),
                serde_json::to_string(&tab("task", "a", "https://a")).unwrap(),
            ),
            ("b".to_string(), "not json".to_string()),
        ]);
        assert_eq!(decode_tabs(fields), vec![tab("task", "a", "https://a")]);
    }
}
