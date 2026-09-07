use super::contracts::{
    EVENT_BUFFER, PreviewTab, PreviewTabDao, PreviewTabError, PreviewTabListing, PreviewTabRemoval,
    PreviewTabTtl,
};
use ::redis::AsyncCommands;
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
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
pub(super) fn decode_tabs(fields: HashMap<String, String>) -> Vec<PreviewTab> {
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
        let mut conn =
            redis::aio::ConnectionManager::new_with_config(client.clone(), redis_manager_config())
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

fn redis_manager_config() -> redis::aio::ConnectionManagerConfig {
    redis::aio::ConnectionManagerConfig::new()
        .set_connection_timeout(Some(std::time::Duration::from_secs(5)))
        .set_response_timeout(Some(std::time::Duration::from_secs(5)))
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
                let _: u64 = conn
                    .del(&tabs)
                    .await
                    .map_err(PreviewTabError::Unavailable)?;
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
