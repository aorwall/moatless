use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

use moatless_core::change_feed::listing::{EnvironmentIdentity, ListingChange, ListingSubject};
use moatless_ports::task::MockTaskReader;
use moatless_types::access_control::ScopeContext;
use serde_json::json;

use super::*;
use crate::config::{AgentModelDefaults, OrchestrationProtocol};
use crate::dao::MockConversationDao;
use crate::feed::ShellFeed;
use crate::reads::T3Reads;
use crate::rpc::envelope::RequestId;
use crate::subscription::StreamSet;
use crate::workspace_listing::MockWorkspaceListing;

fn identity() -> EnvironmentIdentity {
    EnvironmentIdentity::new("7667539899605483559:16384")
}

/// The database the poller and every subscriber read, counting each read.
#[derive(Default)]
struct Database {
    listing_position: AtomicI64,
    conversation_position: AtomicI64,
    listing_heads: AtomicUsize,
    listing_changes: AtomicUsize,
    conversation_heads: AtomicUsize,
    single_conversation_heads: AtomicUsize,
}

impl Database {
    fn dao(self: &Arc<Self>) -> Arc<dyn ConversationDao> {
        let mut dao = MockConversationDao::new();
        let db = self.clone();
        dao.expect_listing_head().returning(move || {
            db.listing_heads.fetch_add(1, Ordering::SeqCst);
            Ok(ListingHead {
                identity: identity(),
                position: db.listing_position.load(Ordering::SeqCst),
            })
        });
        let db = self.clone();
        dao.expect_listing_changes_after()
            .returning(move |after, limit| {
                db.listing_changes.fetch_add(1, Ordering::SeqCst);
                let head = db.listing_position.load(Ordering::SeqCst);
                Ok((after + 1..=head.min(after + limit))
                    .map(|position| ListingChange {
                        position,
                        subject: ListingSubject::Task("listed".to_string()),
                    })
                    .collect())
            });
        let db = self.clone();
        dao.expect_conversation_heads().returning(move |task_ids| {
            db.conversation_heads.fetch_add(1, Ordering::SeqCst);
            let position = db.conversation_position.load(Ordering::SeqCst);
            Ok(task_ids
                .iter()
                .map(|task_id| (task_id.clone(), ConversationHead { position }))
                .collect())
        });
        let db = self.clone();
        dao.expect_conversation_head().returning(move |_| {
            db.single_conversation_heads.fetch_add(1, Ordering::SeqCst);
            Ok(ConversationHead { position: 0 })
        });
        dao.expect_list_followed_by_ids()
            .returning(|_, _| Ok(Vec::new()));
        Arc::new(dao)
    }

    fn reads(self: &Arc<Self>) -> T3Reads {
        let mut workspaces = MockWorkspaceListing::new();
        workspaces
            .expect_list_readable_including()
            .returning(|_, _| Ok(Vec::new()));
        T3Reads::new(
            self.dao(),
            Arc::new(workspaces),
            Arc::new(MockTaskReader::new()),
            crate::poll_gate::PollGate::new(4),
        )
    }

    fn count(counter: &AtomicUsize) -> usize {
        counter.load(Ordering::SeqCst)
    }
}

fn request_id(id: usize) -> RequestId {
    serde_json::from_value(json!(id.to_string())).expect("a string is a request id")
}

/// The reads `subscribers` open shell and thread subscriptions cause over
/// ten seconds, during which the listing moves once.
async fn reads_with(subscribers: usize) -> Arc<Database> {
    let db = Arc::new(Database::default());
    db.listing_position.store(7, Ordering::SeqCst);
    let changes = ChangeWatch::new();
    let cancel = CancellationToken::new();
    let poller = ChangePoller::new(changes.clone(), db.dao()).spawn(cancel.clone());

    let (frames, _pushed) = crate::rpc::outbound::channel(1024);
    let mut streams = StreamSet::new(frames);
    let mut threads = Vec::new();
    for subscriber in 0..subscribers {
        let feed = ShellFeed::resuming_from(identity(), 7).speaking(OrchestrationProtocol::V2);
        let reads = db.reads();
        let listing = changes.listing();
        streams.start_produced(&request_id(subscriber), None, move |sink| {
            crate::rpc::watch::watch_shell(
                sink,
                feed,
                reads,
                Arc::new(AgentModelDefaults::default()),
                ScopeContext::user("viewer"),
                listing,
            )
        });
        threads.push(changes.thread(&format!("task-{subscriber}")));
    }

    tokio::time::sleep(Duration::from_secs(5)).await;
    db.listing_position.store(8, Ordering::SeqCst);
    // Off a tick, so cancelling never races the pass due at the same instant.
    tokio::time::sleep(Duration::from_millis(5_100)).await;
    cancel.cancel();
    poller.await.expect("the poller stops on cancel");
    db
}

#[tokio::test(start_paused = true)]
async fn the_shared_counter_reads_stay_constant_as_subscribers_grow() {
    let one = reads_with(1).await;
    let many = reads_with(64).await;

    let shared = |db: &Database| {
        (
            Database::count(&db.listing_heads),
            Database::count(&db.conversation_heads),
            Database::count(&db.single_conversation_heads),
        )
    };
    assert_eq!(shared(&one), shared(&many));
    assert_eq!(Database::count(&many.single_conversation_heads), 0);
    assert!(Database::count(&many.listing_heads) > 0);
}

#[tokio::test(start_paused = true)]
async fn each_subscriber_reads_its_changes_once_per_move() {
    let db = reads_with(64).await;

    // One read by the poller, to learn which conversations the move named,
    // and one per subscriber, from its own cursor.
    assert_eq!(Database::count(&db.listing_changes), 1 + 64);
}

/// Run one pass of each read against `db`.
async fn pass(poller: &ChangePoller, read_through: &mut Option<ListingHead>) {
    poller
        .poll_listing(read_through)
        .await
        .expect("listing pass");
    poller.poll_threads().await.expect("thread pass");
}

#[tokio::test]
async fn a_watched_conversation_wakes_on_its_head_and_on_its_listing_row() {
    let db = Arc::new(Database::default());
    let changes = ChangeWatch::new();
    let poller = ChangePoller::new(changes.clone(), db.dao());
    let signals = changes.thread("listed");
    let mut read_through = None;

    pass(&poller, &mut read_through).await;
    let first = signals.borrow().expect("read on the first pass");

    db.conversation_position.store(3, Ordering::SeqCst);
    pass(&poller, &mut read_through).await;
    let moved = signals.borrow().expect("still read");
    assert_eq!(moved.head.position, 3);
    assert_eq!(moved.listed, first.listed);

    db.listing_position.store(2, Ordering::SeqCst);
    pass(&poller, &mut read_through).await;
    let listed = signals.borrow().expect("still read");
    assert_eq!(listed.head.position, 3);
    assert_eq!(listed.listed, first.listed + 2);
}

#[tokio::test]
async fn a_conversation_nobody_watches_is_not_read() {
    let db = Arc::new(Database::default());
    let changes = ChangeWatch::new();
    let poller = ChangePoller::new(changes.clone(), db.dao());
    drop(changes.thread("task-1"));

    poller.poll_threads().await.expect("thread pass");

    assert_eq!(Database::count(&db.conversation_heads), 0);
    assert!(changes.watched_threads().is_empty());
}

#[tokio::test]
async fn a_replaced_database_wakes_every_watched_conversation() {
    let db = Arc::new(Database::default());
    let changes = ChangeWatch::new();
    let poller = ChangePoller::new(changes.clone(), db.dao());
    let signals = changes.thread("unnamed");
    let mut read_through = None;
    pass(&poller, &mut read_through).await;
    let before = signals.borrow().expect("read on the first pass");

    read_through = Some(ListingHead {
        identity: EnvironmentIdentity::new("another:1"),
        position: 0,
    });
    pass(&poller, &mut read_through).await;

    assert_eq!(
        signals.borrow().expect("still read").listed,
        before.listed + 1
    );
    assert_eq!(read_through.expect("read").identity, identity());
}

#[tokio::test]
async fn next_change_skips_the_value_already_acted_on() {
    let (sender, mut receiver) = watch::channel(Some(1));

    let waiting = tokio::spawn(async move { next_change(&mut receiver, Some(&1), |_| true).await });
    tokio::task::yield_now().await;
    assert!(!waiting.is_finished());
    sender.send_replace(Some(2));

    assert_eq!(waiting.await.expect("joined"), Some(2));
}

#[tokio::test(start_paused = true)]
async fn a_shell_behind_by_more_than_one_batch_reads_the_rest_without_another_move() {
    let db = Arc::new(Database::default());
    db.listing_position.store(7, Ordering::SeqCst);
    let changes = ChangeWatch::new();
    let cancel = CancellationToken::new();
    let poller = ChangePoller::new(changes.clone(), db.dao()).spawn(cancel.clone());
    let (frames, _pushed) = crate::rpc::outbound::channel(16);
    let mut streams = StreamSet::new(frames);
    let feed = ShellFeed::resuming_from(identity(), 7).speaking(OrchestrationProtocol::V2);
    let reads = db.reads();
    let listing = changes.listing();
    streams.start_produced(&request_id(0), None, move |sink| {
        crate::rpc::watch::watch_shell(
            sink,
            feed,
            reads,
            Arc::new(AgentModelDefaults::default()),
            ScopeContext::user("viewer"),
            listing,
        )
    });

    tokio::time::sleep(Duration::from_millis(1_100)).await;
    // One move of 1,200 changes, then a quiet listing.
    db.listing_position.store(1_207, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(5)).await;
    cancel.cancel();
    poller.await.expect("the poller stops on cancel");

    // Three batches each: 500, 500 and 200, by the poller and the shell.
    assert_eq!(Database::count(&db.listing_changes), 3 + 3);
}
