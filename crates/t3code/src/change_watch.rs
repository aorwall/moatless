//! One replica's reads of the change counters T3 subscriptions wait on.
//!
//! A subscription compares the value published here with the one it last acted
//! on and queries the database only when they differ, from its own cursor, so
//! the counter reads stay constant in the number of open subscriptions. A
//! `watch` keeps only the latest value, and that is enough: no subscriber reads
//! its changes from the value, so one that skips a value misses nothing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use moatless_core::change_feed::conversation::ConversationHead;
use moatless_core::change_feed::listing::ListingHead;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::dao::ConversationDao;
use crate::subscriptions::{SHELL_POLL_INTERVAL, THREAD_POLL_INTERVAL};

/// How many listing changes one pass reads to find the conversations they name;
/// the rest follow on the next pass.
const LISTING_CHANGES_PER_PASS: i64 = 500;

/// Where one watched conversation stands, as last read by this replica.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ThreadSignal {
    pub(crate) head: ConversationHead,
    /// How many listing changes have named this conversation since it was
    /// first watched. Closing, deleting or unfollowing moves the listing and
    /// not the conversation, so this is what wakes a subscription to recheck
    /// access.
    pub(crate) listed: u64,
}

type ThreadSender = watch::Sender<Option<ThreadSignal>>;

/// The values T3 subscriptions on this replica wait on; [`ChangePoller`] writes
/// them.
///
/// `None` until the first read, so a subscriber never mistakes "not read yet"
/// for a position.
#[derive(Clone)]
pub struct ChangeWatch {
    listing: Arc<watch::Sender<Option<ListingHead>>>,
    threads: Arc<Mutex<HashMap<String, ThreadSender>>>,
    /// [`ConversationDao::pull_request_host_changes`], read only while a
    /// receiver lives.
    pull_requests: Arc<watch::Sender<Option<i64>>>,
}

impl ChangeWatch {
    pub(crate) fn new() -> Self {
        Self {
            listing: Arc::new(watch::Sender::new(None)),
            threads: Arc::default(),
            pull_requests: Arc::new(watch::Sender::new(None)),
        }
    }

    /// The pull request host-change count, as of this replica's last read.
    pub(crate) fn pull_requests(&self) -> watch::Receiver<Option<i64>> {
        self.pull_requests.subscribe()
    }

    /// The listing head, as of this replica's last read.
    pub(crate) fn listing(&self) -> watch::Receiver<Option<ListingHead>> {
        self.listing.subscribe()
    }

    /// Where `task_id` stands, read from the next pass on for as long as a
    /// receiver lives.
    pub(crate) fn thread(&self, task_id: &str) -> watch::Receiver<Option<ThreadSignal>> {
        self.lock_threads()
            .entry(task_id.to_string())
            .or_insert_with(|| watch::Sender::new(None))
            .subscribe()
    }

    /// The watched conversations, dropping those whose last receiver is gone.
    fn watched_threads(&self) -> Vec<String> {
        let mut threads = self.lock_threads();
        threads.retain(|_, sender| sender.receiver_count() > 0);
        threads.keys().cloned().collect()
    }

    fn lock_threads(&self) -> std::sync::MutexGuard<'_, HashMap<String, ThreadSender>> {
        // Every critical section leaves the map whole, so a panic in one
        // poisons nothing a later one relies on.
        self.threads.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish_heads(&self, heads: Vec<(String, ConversationHead)>) {
        let threads = self.lock_threads();
        for (task_id, head) in heads {
            if let Some(sender) = threads.get(&task_id) {
                sender.send_if_modified(|signal| match signal {
                    Some(signal) if signal.head == head => false,
                    Some(signal) => {
                        signal.head = head;
                        true
                    }
                    None => {
                        *signal = Some(ThreadSignal { head, listed: 0 });
                        true
                    }
                });
            }
        }
    }

    /// Wake every subscription on a conversation in `task_ids`; `None` wakes
    /// them all.
    fn publish_listed(&self, task_ids: Option<&[String]>) {
        let threads = self.lock_threads();
        let bump = |sender: &ThreadSender| {
            // A conversation not read yet is woken by its first read instead.
            sender.send_if_modified(|signal| {
                signal.as_mut().is_some_and(|signal| {
                    signal.listed += 1;
                    true
                })
            });
        };
        match task_ids {
            Some(task_ids) => task_ids
                .iter()
                .filter_map(|task_id| threads.get(task_id))
                .for_each(bump),
            None => threads.values().for_each(bump),
        }
    }
}

/// Wait until `receiver` holds a value other than `seen` that `wanted` accepts.
///
/// `None` when the sender is gone, which only happens as the process stops.
pub(crate) async fn next_change<T: Clone + PartialEq>(
    receiver: &mut watch::Receiver<Option<T>>,
    seen: Option<&T>,
    wanted: impl Fn(&T) -> bool,
) -> Option<T> {
    let value = receiver
        .wait_for(|value| value.as_ref().is_some_and(|v| Some(v) != seen && wanted(v)))
        .await
        .ok()?;
    value.clone()
}

/// The reading half: polls the counters once per replica until cancelled.
///
/// Reads the database rather than taking a notification, so a change written
/// by another replica arrives the same way as one written here.
pub struct ChangePoller {
    watch: ChangeWatch,
    conversations: Arc<dyn ConversationDao>,
}

impl ChangePoller {
    pub(crate) fn new(watch: ChangeWatch, conversations: Arc<dyn ConversationDao>) -> Self {
        Self {
            watch,
            conversations,
        }
    }

    /// Poll until `cancel`. A failed read is retried on the next tick, and
    /// every subscriber keeps waiting on the value before it.
    pub fn spawn(self, cancel: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!("The T3 UI change poller started");
            self.run(cancel).await;
            info!("The T3 UI change poller stopped");
        })
    }

    async fn run(self, cancel: CancellationToken) {
        let mut listing_ticks = interval(SHELL_POLL_INTERVAL);
        listing_ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut thread_ticks = interval(THREAD_POLL_INTERVAL);
        thread_ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
        // How far this replica has read the listing change log, and in which
        // database.
        let mut listing: Option<ListingHead> = None;
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                _ = listing_ticks.tick() => {
                    if let Err(error) = self.poll_listing(&mut listing).await {
                        warn!(%error, "Failed to read the T3 UI listing head");
                    }
                    if let Err(error) = self.poll_pull_requests().await {
                        warn!(%error, "Failed to read the pull request host-change count");
                    }
                }
                _ = thread_ticks.tick() => {
                    if let Err(error) = self.poll_threads().await {
                        warn!(%error, "Failed to read the watched T3 UI conversation heads");
                    }
                }
            }
        }
    }

    async fn poll_listing(
        &self,
        read_through: &mut Option<ListingHead>,
    ) -> Result<(), sqlx::Error> {
        let head = self.conversations.listing_head().await?;
        match read_through {
            Some(read) if read.identity == head.identity => {
                if read.position < head.position {
                    let after = read.position;
                    let changes = self
                        .conversations
                        .listing_changes_after(after, LISTING_CHANGES_PER_PASS)
                        .await?;
                    let task_ids: Vec<String> = changes
                        .iter()
                        .filter_map(|change| change.task_id().map(str::to_owned))
                        .collect();
                    self.watch.publish_listed(Some(&task_ids));
                    // A pruned window reads empty; the head is then as far as
                    // anything can be read.
                    let through = changes.last().map_or(head.position, |last| last.position);
                    *read_through = Some(ListingHead {
                        identity: head.identity.clone(),
                        position: through,
                    });
                }
            }
            // A replaced database: nothing in the old numbering says which
            // conversations moved, so every one is told to look.
            Some(_) => {
                self.watch.publish_listed(None);
                *read_through = Some(head.clone());
            }
            None => *read_through = Some(head.clone()),
        }
        self.watch.listing.send_if_modified(|published| {
            let moved = published.as_ref() != Some(&head);
            if moved {
                *published = Some(head);
            }
            moved
        });
        Ok(())
    }

    async fn poll_pull_requests(&self) -> Result<(), sqlx::Error> {
        if self.watch.pull_requests.receiver_count() == 0 {
            return Ok(());
        }
        let count = self.conversations.pull_request_host_changes().await?;
        self.watch.pull_requests.send_if_modified(|published| {
            let moved = *published != Some(count);
            *published = Some(count);
            moved
        });
        Ok(())
    }

    async fn poll_threads(&self) -> Result<(), sqlx::Error> {
        let task_ids = self.watch.watched_threads();
        if task_ids.is_empty() {
            return Ok(());
        }
        let heads = self.conversations.conversation_heads(&task_ids).await?;
        self.watch.publish_heads(heads);
        Ok(())
    }
}

#[cfg(test)]
#[path = "change_watch_tests.rs"]
mod tests;
