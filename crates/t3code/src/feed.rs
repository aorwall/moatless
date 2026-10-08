//! Turning recorded changes into the events a watching client applies.
//!
//! Two feeds, over two logs (the sidebar listing and one conversation's
//! transcript), share one rule: the client discards anything at or below the
//! position it already holds, so a cursor is handed back only in the numbering
//! that allocated it. Removals come from what the viewer previously held, never
//! an empty re-read, which would disclose who left; two events needed at one
//! position are sent as a snapshot instead.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};
use tracing::{debug, warn};

use super::{AgentModelDefaults, projection};
use crate::config::OrchestrationProtocol;
use crate::dao::ConversationDao;
use crate::listing::UiRpcEnvironmentIdentity;
use crate::reads::T3Reads;
use moatless_types::access_control::ScopeContext;

use crate::transcript_dao::Transcript;
use moatless_core::change_feed::conversation::{
    ConversationChange, ConversationHead, EVENTS_PER_CHANGE,
};

/// How many recorded changes one poll reads — a cap, not a limit.
///
/// The cursor still advances to the last record read, so a full batch just
/// splits delivery in two; a poll that's always full means the interval is too
/// long.
const CHANGES_PER_POLL: i64 = 500;

/// A client's place in the change feed, the database it's in, and what the
/// client holds.
///
/// The cursor moves over records, not delivered events, and travels with the
/// database identity; all four fields move together.
pub struct ShellFeed {
    identity: UiRpcEnvironmentIdentity,
    cursor: i64,
    threads: HashSet<String>,
    grouped: HashSet<String>,
    /// The wire shape items are built in; positions are the same in both.
    protocol: OrchestrationProtocol,
}

impl ShellFeed {
    /// Start from the position a snapshot was read at, in that database,
    /// holding nothing.
    ///
    /// The removable set stays empty deliberately: a fresh read would miss rows
    /// that left while the client was away.
    pub fn resuming_from(identity: UiRpcEnvironmentIdentity, position: i64) -> Self {
        Self {
            identity,
            cursor: position,
            threads: HashSet::new(),
            grouped: HashSet::new(),
            protocol: OrchestrationProtocol::V1,
        }
    }

    /// Build this feed's items, and the listings it rebuilds, in `protocol`.
    pub(crate) fn speaking(mut self, protocol: OrchestrationProtocol) -> Self {
        self.protocol = protocol;
        self
    }

    /// Seed the Tasks the client holds and the projects it can group them under
    /// from the listing it was sent.
    pub fn holding(mut self, threads: Vec<String>, projects: Vec<String>) -> Self {
        self.threads = threads.into_iter().collect();
        self.grouped = projects.into_iter().collect();
        self
    }

    /// Seed the projects the client can group by. These ids gate Task delivery,
    /// so a wrong one costs a row not rendered until the next rebuild rather
    /// than disclosing a project the feed did not send.
    pub fn grouping_by(mut self, projects: Vec<String>) -> Self {
        self.grouped = projects.into_iter().collect();
        self
    }

    /// Everything this viewer should be told since the last poll, oldest first;
    /// empty means nothing changed or nothing is theirs.
    ///
    /// The head is read too, or a stale subscription would silently skip
    /// everything between the old database and the new one.
    pub async fn poll(
        &mut self,
        reads: &T3Reads,
        defaults: &AgentModelDefaults,
        viewer: &ScopeContext,
    ) -> Result<Vec<Value>, sqlx::Error> {
        let conversations = reads.conversations();
        let workspaces = reads.workspaces();
        let head = conversations.listing_head().await?;
        if head.identity != self.identity {
            return self.rebuild(reads, defaults, viewer).await;
        }

        let changes = conversations
            .listing_changes_after(self.cursor, CHANGES_PER_POLL)
            .await?;
        let Some(last) = changes.last() else {
            return Ok(Vec::new());
        };
        if changes.len() as i64 == CHANGES_PER_POLL {
            debug!(
                cursor = self.cursor,
                through = last.position,
                "T3 UI shell poll filled its batch; the rest follows on the next poll"
            );
        }
        // Where the cursor goes *if this poll finishes*. Every read below is
        // fallible and `watch_shell` survives a failed poll, so a cursor moved
        // past a batch that was never built is a batch nobody is told about.
        let batch_end = last.position;

        // One delivery per Task, at the highest position its burst reached,
        // re-sorted by the position each survivor kept.
        let mut latest: HashMap<&str, i64> = HashMap::new();
        for change in &changes {
            latest
                .entry(change.task_id.as_str())
                .and_modify(|position| *position = (*position).max(change.position))
                .or_insert(change.position);
        }
        let mut coalesced: Vec<(&str, i64)> = latest.into_iter().collect();
        coalesced.sort_by_key(|(id, position)| (*position, *id));

        let task_ids: Vec<String> = coalesced.iter().map(|(id, _)| (*id).to_string()).collect();
        let rows = conversations
            .list_followed_by_ids(viewer, &task_ids)
            .await?;

        // The Workspaces these changed Tasks reference are still needed to
        // project the Task rows and decide whether the client can group them.
        let wanted: Vec<String> = rows
            .iter()
            .filter_map(|row| row.workspace_id.clone())
            .collect();
        let projects = workspaces.list_readable_including(viewer, &wanted).await?;

        let by_id: HashMap<&str, &_> = rows.iter().map(|row| (row.task_id.as_str(), row)).collect();
        let mut items = Vec::new();
        let mut delivered_threads: Vec<&str> = Vec::new();
        let mut dropped_threads: Vec<&str> = Vec::new();

        for (id, position) in coalesced {
            let row = by_id.get(id);
            let project_id = row.and_then(|row| {
                row.workspace_id
                    .as_deref()
                    .filter(|id| projects.iter().any(|project| project.workspace.id == *id))
            });
            match (row, project_id) {
                (Some(row), Some(project_id)) => {
                    if !self.grouped.contains(project_id) {
                        // Entered a project this client has no group for, and
                        // the pair cannot go out at one position.
                        return self.resynchronize(reads, defaults, viewer).await;
                    }
                    items.push(match self.protocol {
                        OrchestrationProtocol::V1 => {
                            projection::thread_upserted(row, project_id, defaults, position)
                        }
                        OrchestrationProtocol::V2 => {
                            projection::thread_updated_v2(row, project_id, defaults, position)
                        }
                    });
                    delivered_threads.push(id);
                }
                // Followed and readable, but its project is not listable; the
                // client would drop it in silence, so a holder is told.
                (Some(row), None) => {
                    warn!(
                        task_id = %row.task_id,
                        workspace_id = ?row.workspace_id,
                        "Changed conversation has no listable workspace; \
                         not sending it to the T3 shell"
                    );
                    if self.threads.contains(id) {
                        items.push(self.removed(id, position));
                        dropped_threads.push(id);
                    }
                }
                // Unfollowed, no longer readable, or gone. From what this
                // client holds, never from the empty re-read. A conversation
                // that was archived is not here — it comes back present,
                // carrying `archivedAt`, because a resumed feed holds nothing
                // and would drop the removal.
                (None, _) => {
                    if self.threads.contains(id) {
                        items.push(self.removed(id, position));
                        dropped_threads.push(id);
                    }
                }
            }
        }

        for id in delivered_threads {
            self.threads.insert(id.to_string());
        }
        for id in dropped_threads {
            self.threads.remove(id);
        }
        // Last, with the items built and nothing fallible left between here and
        // the caller. See `batch_end`.
        self.cursor = batch_end;
        Ok(items)
    }

    fn removed(&self, task_id: &str, position: i64) -> Value {
        match self.protocol {
            OrchestrationProtocol::V1 => projection::thread_removed(task_id, position),
            OrchestrationProtocol::V2 => projection::thread_removed_v2(task_id, position),
        }
    }

    /// Put a watching client back onto a listing built from the database that
    /// is actually there now.
    ///
    /// The cursor is *set*, not advanced like [`ShellFeed::resynchronize`] —
    /// the old counter is gone, and `max()`-ing it in would park the feed
    /// silent.
    async fn rebuild(
        &mut self,
        reads: &T3Reads,
        defaults: &AgentModelDefaults,
        viewer: &ScopeContext,
    ) -> Result<Vec<Value>, sqlx::Error> {
        let snapshot =
            crate::snapshots::shell_snapshot_in(reads, defaults, viewer, self.protocol).await?;
        warn!(
            user_id = %viewer.user_id,
            cursor = self.cursor,
            sequence = snapshot.sequence,
            "The database under this deployment was replaced while a T3 UI shell \
             subscription was open; rebuilding its listing"
        );
        let cursor = snapshot.sequence;
        Ok(self.adopt(snapshot, cursor))
    }

    /// Rebuild the listing and send it whole — the one delivery that can change
    /// what a sidebar is grouped by, since the client replaces its snapshot
    /// without comparing positions.
    async fn resynchronize(
        &mut self,
        reads: &T3Reads,
        defaults: &AgentModelDefaults,
        viewer: &ScopeContext,
    ) -> Result<Vec<Value>, sqlx::Error> {
        let snapshot =
            crate::snapshots::shell_snapshot_in(reads, defaults, viewer, self.protocol).await?;
        debug!(
            user_id = %viewer.user_id,
            sequence = snapshot.sequence,
            "A conversation entered a project the T3 UI shell client does not hold; \
             sending a fresh listing"
        );
        // Advanced, not set — the counter is the same counter, so the old cursor
        // is still meaningful and rewinding it would redeliver. The snapshot
        // covers everything up to the position it was read at, which is at or
        // past this poll's batch. Contrast `rebuild`.
        let cursor = self.cursor.max(snapshot.sequence);
        Ok(self.adopt(snapshot, cursor))
    }

    /// Take a freshly built listing as the client's whole state and hand it
    /// over: both callers replace what the client is looking at, so this
    /// replaces everything the feed believes about its Tasks and grouping.
    fn adopt(&mut self, snapshot: super::ShellSnapshot, cursor: i64) -> Vec<Value> {
        self.threads = snapshot.thread_ids().into_iter().collect();
        self.grouped = snapshot.project_ids().into_iter().collect();
        self.identity = snapshot.identity;
        self.cursor = cursor;
        vec![json!({
            "kind": "snapshot",
            "snapshot": snapshot.value
        })]
    }
}

/// A watching client's place in one conversation.
///
/// Reads forward from the client's position in this conversation's change log;
/// a poll coalesces a burst per Message (see [`coalesced`]).
pub struct ThreadFeed {
    task_id: String,
    /// The position this feed reads from next, in the change log's own
    /// numbering rather than the contract's.
    cursor: i64,
    /// The sequence the client has been sent, below which an event of a re-read
    /// change is dropped rather than sent again. See [`ThreadFeed::poll`].
    delivered: i64,
    /// The newest turn the client holds a checkpoint for; a settling turn sends
    /// every settled turn above it. `None` when unknown, as on a resume, which
    /// sends them all.
    checkpointed: Option<i32>,
    /// Those turns outnumber one change's slots, so the caller must answer with
    /// a fresh snapshot: see [`crate::subscriptions::poll_thread`].
    needs_snapshot: bool,
}

/// How many recorded changes one poll reads: a cap, not a window — the rest is
/// read by the next poll, so a client far behind catches up over several
/// intervals rather than one large frame.
const THREAD_POLL_CHANGES: i64 = 512;

impl ThreadFeed {
    /// Start from the position a snapshot was built at. The client holds
    /// everything that position covers, so the first live poll is looking for
    /// what came after it.
    pub fn at(task_id: String, head: ConversationHead, checkpointed: Option<i32>) -> Self {
        Self {
            task_id,
            cursor: head.position + 1,
            delivered: head.sequence(),
            checkpointed,
            needs_snapshot: false,
        }
    }

    /// Start from the sequence a returning client says it holds.
    ///
    /// The cursor is the position that sequence sits *inside*, not after it — a
    /// mid-change disconnect re-reads the rest, filtered at `after`.
    pub fn resuming_from(task_id: String, after: i64) -> Self {
        Self {
            task_id,
            cursor: moatless_core::change_feed::conversation::position_of(after),
            delivered: after,
            checkpointed: None,
            needs_snapshot: false,
        }
    }

    fn log_unreadable(&self, viewer: &ScopeContext) {
        debug!(
            task_id = %self.task_id,
            user_id = %viewer.user_id,
            "The conversation a T3 UI thread subscription watches is no longer visible to it"
        );
    }

    pub(crate) fn task_id(&self) -> &str {
        &self.task_id
    }

    /// Whether a poll found more settled turns than it could send; cleared
    /// only by replacing the feed with one at a fresh snapshot.
    pub(crate) fn needs_snapshot(&self) -> bool {
        self.needs_snapshot
    }

    /// Everything recorded since this feed last read, as stream items — empty
    /// when nothing changed, `None` when gone.
    ///
    /// Rows re-read once per batch may go stale mid-poll; the cursor advances
    /// only once every read succeeds, so a failed poll retries from here.
    pub(crate) async fn poll(
        &mut self,
        conversations: &dyn ConversationDao,
        viewer: &ScopeContext,
        frontend_url: &str,
    ) -> Result<Option<Vec<Value>>, sqlx::Error> {
        let changes = conversations
            .conversation_changes(&self.task_id, self.cursor, THREAD_POLL_CHANGES)
            .await?;

        // A silent poll still checks access: closing or deleting a Task records
        // no conversation change, so a check run only on changes would never end
        // this subscription. It asks `can_read_conversation` rather than
        // `readable_by_id`, whose listing query, once a second per open thread,
        // held a large share of the pool.
        let Some(last) = changes.last() else {
            if !conversations
                .can_read_conversation(viewer, &self.task_id)
                .await?
            {
                self.log_unreadable(viewer);
                return Ok(None);
            }
            return Ok(Some(Vec::new()));
        };

        // The conversation itself, through the same read the snapshot resolved
        // it with, so a conversation that left this viewer's reach needs no rule
        // repeated here. By access rather than by Follow, or an open
        // subscription would end on its first poll for every reader who opened
        // a conversation they do not follow.
        let Some(conversation) = conversations.readable_by_id(viewer, &self.task_id).await? else {
            self.log_unreadable(viewer);
            return Ok(None);
        };
        let read_through = last.position;

        let changes = coalesced(changes);
        let named: Vec<String> = changes
            .iter()
            .filter_map(|change| change.message_uuid.clone())
            .collect();
        let named_activities: Vec<String> = changes
            .iter()
            .filter_map(|change| change.activity_id.clone())
            .collect();
        let window = conversations
            .changed_messages(&self.task_id, &named, &named_activities)
            .await?;

        let origins = projection::OriginContext {
            frontend_url,
            source_task_names: &super::source_task_names(conversations, viewer, &window).await,
        };

        // Read only when the batch holds a session change, the one a settling
        // turn records, and sent after the last of them alone. A rewind before
        // it lowers the floor first: the turns it cut are numbered again.
        let last_session_change = changes.iter().rposition(projection::is_session_change);
        let settled = match last_session_change {
            Some(last) => {
                let floor = changes[..last]
                    .iter()
                    .filter_map(|change| change.reverted_to_turn)
                    .fold(self.checkpointed, |floor, turn| floor.min(Some(turn)));
                let settled = settled_since(conversations, &self.task_id, floor).await?;
                if settled.turns.len() > CHECKPOINT_SLOTS {
                    self.needs_snapshot = true;
                    None
                } else {
                    Some(settled)
                }
            }
            None => None,
        };

        let mut items = Vec::new();
        for (index, change) in changes.iter().enumerate() {
            if let Some(turn) = change.reverted_to_turn {
                self.checkpointed = self.checkpointed.min(Some(turn));
            }
            let mut settled_events = Vec::new();
            if let Some(settled) = settled
                .as_ref()
                .filter(|_| Some(index) == last_session_change)
            {
                for (turn, event) in projection::turn_settled_events(&conversation, change, settled)
                {
                    self.checkpointed = self.checkpointed.max(Some(turn));
                    settled_events.push(event);
                }
            }
            let events = projection::change_events(&conversation, change, &window, &origins);
            for item in events.into_iter().chain(settled_events) {
                // An event the client has already passed is dropped by it in
                // silence, so it is dropped here instead: only a resuming feed
                // ever produces one, and only for the change its cursor sits
                // inside.
                let sequence = item["event"]["sequence"].as_i64().unwrap_or_default();
                if sequence > self.delivered {
                    self.delivered = sequence;
                    items.push(item);
                }
            }
        }

        // Last, and only once every fallible read above has returned. A failed
        // poll is retried by `watch_thread` on the stated ground that the cursor
        // has not moved, so the next poll covers the same ground — advancing it
        // before the reads that can fail makes that sentence false and loses the
        // batch outright, with nothing logged on the client's side and nothing
        // wrong on its screen except that the conversation has stopped.
        self.cursor = read_through + 1;
        Ok(Some(items))
    }
}

/// How many settled turns [`settled_since`] reads per page.
const SETTLED_TURNS_PAGE: i64 = 64;

/// The slots one change has for checkpoints, after its `thread.session-set`.
const CHECKPOINT_SLOTS: usize = EVENTS_PER_CHANGE.unsigned_abs() as usize - 1;

/// Every settled turn above `floor`, oldest first, paged until the gap drains
/// or outnumbers [`CHECKPOINT_SLOTS`].
async fn settled_since(
    conversations: &dyn ConversationDao,
    task_id: &str,
    mut floor: Option<i32>,
) -> Result<Transcript, sqlx::Error> {
    let mut settled = Transcript::default();
    loop {
        let page = conversations
            .settled_turns(task_id, floor, SETTLED_TURNS_PAGE)
            .await?;
        let drained = page.turns.len() < usize::try_from(SETTLED_TURNS_PAGE).unwrap_or(0);
        floor = page.turns.last().copied().or(floor);
        settled.messages.extend(page.messages);
        settled.turns.extend(page.turns);
        if drained || settled.turns.len() > CHECKPOINT_SLOTS {
            return Ok(settled);
        }
    }
}

/// One change per named row, kept at its last position, in position order.
///
/// Changes naming neither a Message uuid nor an activity id are the
/// conversation's own and are all kept as ordered transitions.
fn coalesced(changes: Vec<ConversationChange>) -> Vec<ConversationChange> {
    let mut latest: HashMap<&str, i64> = HashMap::new();
    for change in &changes {
        if let Some(name) = change
            .message_uuid
            .as_deref()
            .or(change.activity_id.as_deref())
        {
            latest.insert(name, change.position);
        }
    }
    let keep: HashSet<i64> = latest.into_values().collect();
    changes
        .iter()
        .filter(|change| {
            change
                .message_uuid
                .as_deref()
                .or(change.activity_id.as_deref())
                .is_none_or(|_| keep.contains(&change.position))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dao::{Conversation, MockConversationDao};
    use crate::transcript_dao::{Transcript, TranscriptMessage};
    use moatless_core::change_feed::conversation::EVENTS_PER_CHANGE;
    use moatless_types::agent::AgentProfile;
    use moatless_types::message::{MessageRole, MessageType};
    use moatless_types::sandbox::SandboxDesiredState;

    fn change(position: i64, uuid: Option<&str>) -> ConversationChange {
        ConversationChange {
            position,
            message_uuid: uuid.map(str::to_string),
            activity_id: None,
            reverted_to_turn: None,
        }
    }

    fn activity_change(position: i64, activity_id: &str) -> ConversationChange {
        ConversationChange {
            position,
            message_uuid: None,
            activity_id: Some(activity_id.to_string()),
            reverted_to_turn: None,
        }
    }

    fn positions(changes: &[ConversationChange]) -> Vec<i64> {
        changes.iter().map(|change| change.position).collect()
    }

    /// One Message is assembled from its blocks by a run of in-place merges,
    /// and each of them records a change naming it. Re-reading that Message
    /// once per change would send the same bubble several times over.
    #[test]
    fn a_burst_naming_one_message_is_read_once_at_its_last_position() {
        let coalesced = coalesced(vec![
            change(1, Some("msg-a")),
            change(2, Some("msg-a")),
            change(3, Some("msg-b")),
            change(4, Some("msg-a")),
        ]);
        assert_eq!(positions(&coalesced), vec![3, 4]);
    }

    /// Each is a distinct transition of the conversation's session, and the
    /// client derives the turn it renders from the last one it is sent.
    #[test]
    fn changes_naming_no_message_are_all_kept() {
        let coalesced = coalesced(vec![
            change(1, None),
            change(2, None),
            change(3, Some("msg-a")),
        ]);
        assert_eq!(positions(&coalesced), vec![1, 2, 3]);
    }

    /// A Sandbox row restated in place folds like a Message: the last position
    /// is the one read, so the row is re-sent once rather than once per change.
    #[test]
    fn a_burst_naming_one_activity_is_read_once_at_its_last_position() {
        let coalesced = coalesced(vec![
            activity_change(1, "evt-a"),
            activity_change(2, "evt-a"),
            activity_change(3, "evt-b"),
            activity_change(4, "evt-a"),
        ]);
        assert_eq!(positions(&coalesced), vec![3, 4]);
    }

    /// A change naming neither a Message nor an activity is the conversation's
    /// own, and stands beside the folded activity changes untouched.
    #[test]
    fn a_change_naming_neither_survives_beside_folded_activities() {
        let coalesced = coalesced(vec![
            activity_change(1, "evt-a"),
            change(2, None),
            activity_change(3, "evt-a"),
        ]);
        assert_eq!(positions(&coalesced), vec![2, 3]);
    }

    /// A client holding a snapshot holds everything its position covers, so the
    /// first live poll is looking for what came after it — and nothing the
    /// snapshot already showed may be sent again under a lower sequence.
    #[test]
    fn a_feed_started_from_a_snapshot_reads_past_the_position_it_was_built_at() {
        let feed = ThreadFeed::at("task-a".into(), ConversationHead { position: 7 }, None);
        assert_eq!(feed.cursor, 8);
        assert_eq!(feed.delivered, 7 * EVENTS_PER_CHANGE);
    }

    /// A client disconnected partway through one change's events holds a
    /// sequence inside that change, and what it missed is the rest of that same
    /// change — so the cursor is the position it sits in, not the next one.
    #[test]
    fn a_resuming_feed_reads_the_change_its_cursor_sits_inside() {
        let inside = 7 * EVENTS_PER_CHANGE + 3;
        let feed = ThreadFeed::resuming_from("task-a".into(), inside);
        assert_eq!(feed.cursor, 7);
        assert_eq!(feed.delivered, inside);
    }

    fn a_conversation() -> Conversation {
        Conversation {
            task_id: "task-a".into(),
            workspace_id: Some("ws-a".into()),
            name: Some("Fix the build".into()),
            branch: None,
            agent_profile: AgentProfile::ClaudeCode.into(),
            created_at: "2026-07-01T09:00:00.000Z".into(),
            created_at_db: None,
            updated_at: None,
            closed_at: None,
            visibility: moatless_types::task::Visibility::Private,
            owner_name: None,
            turn_number: Some(3),
            turn_status: Some("active".into()),
            turn_started_at: None,
            turn_ended_at: None,
            turn_error: None,
            sandbox_desired_state_raw: Some(SandboxDesiredState::Running.as_str().to_string()),
            sandbox_stop_reason_raw: None,
            sandbox_started_at: None,
            sandbox_updated_at: None,
            running_commands: 0,
            latest_user_message_at: None,
            latest_queued_message_at: None,
            assistant_model: None,
            requested_model: None,
            pending_question: false,
            pending_plan: false,
            parent_task_id: None,
            parent_relation: None,
            fork_at_turn: None,
            settled_override: None,
            settled_at: None,
            snoozed_until: None,
            snoozed_at: None,
            pull_requests: None,
        }
    }

    fn a_message(id: i32, uuid: &str) -> TranscriptMessage {
        TranscriptMessage {
            id,
            uuid: uuid.into(),
            message_type: MessageType::User,
            message_role: MessageRole::User,
            message_text: Some("Fix the build".into()),
            user_type: "human".into(),
            sandbox_wake: false,
            held: false,
            turn_number: Some(3),
            timestamp: "2026-07-01T10:00:00.000Z".into(),
            created_at: None,
            external_adapter: None,
            external_source_name: None,
            external_subject_key: None,
            external_event_id: None,
            external_author: None,
            source_task_id: None,
            binding_url: None,
            binding_label: None,
            sent_by_name: None,
            file_contexts: None,
        }
    }

    /// One change naming one Message, over a DAO that answers the three reads a
    /// poll makes.
    fn a_dao(changes: Vec<ConversationChange>) -> MockConversationDao {
        let mut dao = MockConversationDao::new();
        dao.expect_conversation_changes()
            .returning(move |_, _, _| Ok(changes.clone()));
        dao.expect_readable_by_id()
            .returning(|_, _| Ok(Some(a_conversation())));
        dao.expect_changed_messages().returning(|_, _, _| {
            Ok(Transcript {
                messages: vec![a_message(11, "msg-a")],
                activities: Vec::new(),
                omitted: 0,
                turns: Vec::new(),
                settled_requests: HashSet::new(),
                subagent_usage: HashMap::new(),
            })
        });
        dao
    }

    fn viewer() -> ScopeContext {
        ScopeContext {
            user_id: "user-1".into(),
            is_admin: false,
        }
    }

    /// The whole point of the change log: a conversation that moves costs the
    /// row that moved, as the contract's own event, rather than the transcript.
    #[tokio::test]
    async fn a_poll_sends_events_rather_than_the_conversation_again() {
        let dao = a_dao(vec![change(9, Some("msg-a"))]);
        let mut feed = ThreadFeed::at("task-a".into(), ConversationHead { position: 8 }, None);
        let items = feed
            .poll(&dao, &viewer(), "http://localhost:3000")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["kind"], "event");
        assert_eq!(items[0]["event"]["type"], "thread.message-sent");
        assert_eq!(items[0]["event"]["payload"]["messageId"], "msg-a");
        assert_eq!(
            items[0]["event"]["sequence"].as_i64().unwrap(),
            9 * EVENTS_PER_CHANGE,
            "numbered in the change log's own namespace"
        );
        assert_eq!(feed.cursor, 10, "the cursor advanced past the change read");
    }

    /// A feed over `changes` in a conversation whose turn `newest` has settled;
    /// the caller sets what `settled_turns` answers.
    fn settling_dao(changes: Vec<ConversationChange>, newest: i32) -> MockConversationDao {
        let mut dao = MockConversationDao::new();
        dao.expect_conversation_changes()
            .returning(move |_, _, _| Ok(changes.clone()));
        dao.expect_changed_messages()
            .returning(|_, _, _| Ok(Transcript::default()));
        dao.expect_readable_by_id().returning(move |_, _| {
            Ok(Some(Conversation {
                turn_number: Some(newest),
                turn_status: Some("completed".into()),
                ..a_conversation()
            }))
        });
        dao
    }

    fn settled(turns: Vec<i32>, messages: Vec<TranscriptMessage>) -> Transcript {
        Transcript {
            messages,
            turns,
            ..Transcript::default()
        }
    }

    fn reverted(position: i64, turn: i32) -> ConversationChange {
        ConversationChange {
            reverted_to_turn: Some(turn),
            ..change(position, None)
        }
    }

    /// Each checkpoint event's turn id, in the order sent.
    fn checkpointed_turns(items: &[Value]) -> Vec<&str> {
        items
            .iter()
            .filter(|item| item["event"]["type"] == "thread.turn-diff-completed")
            .map(|item| item["event"]["payload"]["turnId"].as_str().unwrap())
            .collect()
    }

    /// A turn settling records a session change, and a page open on it is sent
    /// every turn settled above the one it holds a checkpoint for, after the
    /// session — else the rewind control reaches those turns only on reload.
    #[tokio::test]
    async fn settling_turns_send_their_checkpoints_after_the_session() {
        let mut dao = settling_dao(vec![change(8, None), change(9, None)], 4);
        let answer = |id: i32, uuid: &str, turn: i32| {
            let mut answer = a_message(id, uuid);
            answer.message_type = MessageType::Assistant;
            answer.message_role = MessageRole::Assistant;
            answer.turn_number = Some(turn);
            answer
        };
        let answers = vec![answer(12, "msg-b", 3), answer(13, "msg-c", 4)];
        dao.expect_settled_turns()
            .withf(|_, above, _| *above == Some(2))
            .times(1)
            .returning(move |_, _, _| Ok(settled(vec![3, 4], answers.clone())));
        let mut feed = ThreadFeed::at("task-a".into(), ConversationHead { position: 7 }, Some(2));
        let items = feed
            .poll(&dao, &viewer(), "http://localhost:3000")
            .await
            .unwrap()
            .unwrap();

        let sent: Vec<(&str, i64, &str)> = items
            .iter()
            .map(|item| {
                let event = &item["event"];
                (
                    event["type"].as_str().unwrap(),
                    event["sequence"].as_i64().unwrap(),
                    event["payload"]["assistantMessageId"]
                        .as_str()
                        .unwrap_or_default(),
                )
            })
            .collect();
        assert_eq!(
            sent,
            [
                ("thread.session-set", 8 * EVENTS_PER_CHANGE, ""),
                ("thread.session-set", 9 * EVENTS_PER_CHANGE, ""),
                (
                    "thread.turn-diff-completed",
                    9 * EVENTS_PER_CHANGE + 1,
                    "msg-b"
                ),
                (
                    "thread.turn-diff-completed",
                    9 * EVENTS_PER_CHANGE + 2,
                    "msg-c"
                ),
            ]
        );
        assert_eq!(feed.checkpointed, Some(4));
    }

    /// A rewind deletes the turns above the one kept, and the next run numbers
    /// its turns again from there: they are read from the kept turn, not from
    /// the checkpoint the client held before the rewind.
    #[tokio::test]
    async fn a_rewind_lowers_the_floor_to_the_kept_turn() {
        let mut dao = settling_dao(vec![reverted(8, 2), change(9, None)], 3);
        dao.expect_settled_turns()
            .withf(|_, above, _| *above == Some(2))
            .times(1)
            .returning(|_, _, _| Ok(settled(vec![3], Vec::new())));
        let mut feed = ThreadFeed::at("task-a".into(), ConversationHead { position: 7 }, Some(4));
        let items = feed
            .poll(&dao, &viewer(), "http://localhost:3000")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(checkpointed_turns(&items), ["task-a:3"]);
        assert_eq!(feed.checkpointed, Some(3));
    }

    /// A gap wider than one read is paged oldest first, so no turn below the
    /// newest sent is skipped.
    #[tokio::test]
    async fn a_gap_wider_than_a_page_is_read_to_its_end() {
        let first: Vec<i32> = (1..=i32::try_from(SETTLED_TURNS_PAGE).unwrap()).collect();
        let last = *first.last().unwrap();
        let mut dao = settling_dao(vec![change(9, None)], last + 2);
        dao.expect_settled_turns()
            .withf(|_, above, _| above.is_none())
            .times(1)
            .returning(move |_, _, _| Ok(settled(first.clone(), Vec::new())));
        dao.expect_settled_turns()
            .withf(move |_, above, _| *above == Some(last))
            .times(1)
            .returning(move |_, _, _| Ok(settled(vec![last + 1, last + 2], Vec::new())));
        let mut feed = ThreadFeed::resuming_from("task-a".into(), 8 * EVENTS_PER_CHANGE);
        let items = feed
            .poll(&dao, &viewer(), "http://localhost:3000")
            .await
            .unwrap()
            .unwrap();

        let turns = checkpointed_turns(&items);
        assert_eq!(turns.len(), usize::try_from(last + 2).unwrap());
        assert_eq!(turns.first(), Some(&"task-a:1"));
        assert_eq!(feed.checkpointed, Some(last + 2));
    }

    /// More settled turns than one change has slots for cannot all be sent as
    /// checkpoints, and no later change is promised to carry the rest: the
    /// feed sends none of them and asks for a fresh snapshot instead.
    #[tokio::test]
    async fn more_settled_turns_than_slots_ask_for_a_snapshot() {
        let total = i32::try_from(EVENTS_PER_CHANGE).unwrap();
        let mut dao = settling_dao(vec![change(9, None)], total);
        dao.expect_settled_turns()
            .returning(move |_, above, limit| {
                let from = above.unwrap_or(0) + 1;
                let to = (from + i32::try_from(limit).unwrap() - 1).min(total);
                Ok(settled((from..=to).collect(), Vec::new()))
            });
        let mut feed = ThreadFeed::resuming_from("task-a".into(), 9 * EVENTS_PER_CHANGE);
        let items = feed
            .poll(&dao, &viewer(), "http://localhost:3000")
            .await
            .unwrap()
            .unwrap();

        assert!(checkpointed_turns(&items).is_empty(), "{items:#?}");
        assert!(feed.needs_snapshot());
        assert_eq!(feed.checkpointed, None);
        assert_eq!(feed.cursor, 10);
    }

    /// The client drops an event at or below the position it holds, in silence.
    /// A resuming feed re-reads the change its cursor sits inside, so it is
    /// this side that has to drop the events the client already has.
    #[tokio::test]
    async fn a_resuming_poll_drops_the_events_the_client_already_holds() {
        let dao = a_dao(vec![change(9, Some("msg-a"))]);
        let already = 9 * EVENTS_PER_CHANGE;
        let mut feed = ThreadFeed::resuming_from("task-a".into(), already);
        let items = feed
            .poll(&dao, &viewer(), "http://localhost:3000")
            .await
            .unwrap()
            .unwrap();
        assert!(
            items.is_empty(),
            "nothing above the client's position: {items:#?}"
        );
        assert_eq!(feed.delivered, already);
    }

    /// A conversation that left this viewer's reach ends the subscription — one
    /// answer for deleted, unfollowed and no longer readable.
    #[tokio::test]
    async fn a_poll_that_can_no_longer_see_the_conversation_ends_the_subscription() {
        let mut dao = MockConversationDao::new();
        dao.expect_conversation_changes()
            .returning(|_, _, _| Ok(vec![change(9, Some("msg-a"))]));
        dao.expect_readable_by_id().returning(|_, _| Ok(None));
        let mut feed = ThreadFeed::at("task-a".into(), ConversationHead { position: 8 }, None);
        assert!(
            feed.poll(&dao, &viewer(), "http://localhost:3000")
                .await
                .unwrap()
                .is_none()
        );
    }

    /// An idle conversation is not a gone one, and a subscription that ended on
    /// silence would end every one of them. The access check is the one-row
    /// read: the full row costs the listing query, once a second per open
    /// thread.
    #[tokio::test]
    async fn a_poll_over_a_conversation_that_did_not_move_says_nothing() {
        let mut dao = MockConversationDao::new();
        dao.expect_conversation_changes()
            .returning(|_, _, _| Ok(Vec::new()));
        dao.expect_can_read_conversation()
            .times(1)
            .returning(|_, _| Ok(true));
        dao.expect_readable_by_id().never();
        let mut feed = ThreadFeed::at("task-a".into(), ConversationHead { position: 8 }, None);
        assert_eq!(
            feed.poll(&dao, &viewer(), "http://localhost:3000")
                .await
                .unwrap(),
            Some(Vec::new())
        );
        assert_eq!(feed.cursor, 9, "and the cursor stayed where it was");
    }

    /// Closing, deleting or unfollowing a Task records no change, so a check
    /// that only ran when something was recorded would never catch it — which
    /// is why the silent poll above still checks access.
    #[tokio::test]
    async fn an_idle_subscription_still_discovers_that_its_conversation_ended() {
        let mut dao = MockConversationDao::new();
        dao.expect_conversation_changes()
            .returning(|_, _, _| Ok(Vec::new()));
        dao.expect_can_read_conversation()
            .returning(|_, _| Ok(false));
        dao.expect_readable_by_id().never();
        let mut feed = ThreadFeed::at("task-a".into(), ConversationHead { position: 8 }, None);
        assert!(
            feed.poll(&dao, &viewer(), "http://localhost:3000")
                .await
                .unwrap()
                .is_none(),
            "a conversation that ended without recording a change still ends the subscription"
        );
    }

    /// `watch_thread` retries a failed poll because the cursor hasn't moved. If
    /// the cursor advanced before the reads that can fail, that would be false
    /// and the batch would be lost silently.
    #[tokio::test]
    async fn a_poll_that_fails_part_way_leaves_the_cursor_where_it_started() {
        let mut dao = MockConversationDao::new();
        dao.expect_conversation_changes()
            .returning(|_, _, _| Ok(vec![change(9, Some("msg-a"))]));
        dao.expect_readable_by_id()
            .returning(|_, _| Ok(Some(a_conversation())));
        // The read that comes after the cursor would have moved.
        dao.expect_changed_messages()
            .returning(|_, _, _| Err(sqlx::Error::PoolTimedOut));

        let mut feed = ThreadFeed::at("task-a".into(), ConversationHead { position: 8 }, None);
        assert!(
            feed.poll(&dao, &viewer(), "http://localhost:3000")
                .await
                .is_err()
        );
        assert_eq!(
            feed.cursor, 9,
            "the next poll has to read change 9 again, or it is never sent"
        );
    }
}
