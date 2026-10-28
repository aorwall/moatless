//! One replica's reads of `record_collections.record_seq` for every
//! collection a page connected here is subscribed to.
//!
//! One query per pass covers every watched collection, so the reads stay
//! constant in the number of open sockets. A `watch` keeps only the latest
//! value, and that is enough: a socket reads the changes themselves from
//! `collection_record_changes` after its own cursor, never from the value.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::dao::RecordDao;

/// How long a reader's write takes, at most, to reach a page on another
/// replica, past the database round trips.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// The latest `record_seq` per watched collection; 0 until the first read.
#[derive(Default)]
pub(crate) struct RecordWatch {
    heads: Mutex<HashMap<String, watch::Sender<i64>>>,
}

impl RecordWatch {
    /// A receiver that changes whenever the collection's `record_seq` does.
    /// The collection stays watched while any receiver of it is held.
    pub(crate) fn subscribe(&self, collection_id: &str) -> watch::Receiver<i64> {
        self.heads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(collection_id.to_string())
            .or_insert_with(|| watch::Sender::new(0))
            .subscribe()
    }

    /// The collections still subscribed to, dropping those no receiver holds.
    fn watched(&self) -> Vec<String> {
        let mut heads = self.heads.lock().unwrap_or_else(PoisonError::into_inner);
        heads.retain(|_, sender| sender.receiver_count() > 0);
        heads.keys().cloned().collect()
    }

    fn publish(&self, read: Vec<(String, i64)>) {
        let heads = self.heads.lock().unwrap_or_else(PoisonError::into_inner);
        for (collection_id, seq) in read {
            if let Some(sender) = heads.get(&collection_id) {
                sender.send_if_modified(|head| std::mem::replace(head, seq) != seq);
            }
        }
    }

    /// Read every watched head each [`POLL_INTERVAL`] until `cancel`. A failed
    /// read is logged and retried on the next pass.
    pub(crate) async fn run(&self, store: &dyn RecordDao, cancel: CancellationToken) {
        let mut ticks = interval(POLL_INTERVAL);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                _ = ticks.tick() => {}
            }
            let watched = self.watched();
            if watched.is_empty() {
                continue;
            }
            match store.record_heads(&watched).await {
                Ok(read) => self.publish(read),
                Err(error) => warn!(%error, "Failed to read record collection heads"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_receiver_hears_a_new_head_and_a_dropped_one_stops_the_watch() {
        let watch = RecordWatch::default();
        let mut receiver = watch.subscribe("art-1");
        watch.publish(vec![("art-1".to_string(), 3), ("art-2".to_string(), 9)]);
        assert!(receiver.has_changed().unwrap());
        assert_eq!(*receiver.borrow_and_update(), 3);
        watch.publish(vec![("art-1".to_string(), 3)]);
        assert!(!receiver.has_changed().unwrap());
        assert_eq!(watch.watched(), vec!["art-1".to_string()]);
        drop(receiver);
        assert!(watch.watched().is_empty());
    }
}
