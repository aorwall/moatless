//! Turning recorded changes into the events a watching client applies.
//!
//! Two feeds over two logs — the sidebar's listing and one conversation's
//! transcript — and one rule they share: the client folds events into the
//! snapshot it holds and discards anything at or below the position it already
//! has, so a cursor is only ever handed back in the numbering that allocated it.
//! The notes below are the *shell's*; [`ThreadFeed`] carries its own.
//!
//! The shell side never sends a difference:
//! it coalesces a burst per conversation, re-reads the row through the same
//! listing query the snapshot uses, and sends it whole at the burst's last
//! position. Removals come from what the viewer **previously held**, never from
//! an empty re-read, which on a shared deployment would disclose the id of every
//! conversation unfollowed, made private or deleted (Decision 11).
//!
//! One case is not expressible as a delta: a conversation entering a project the
//! client does not hold needs both events, and two events at one position
//! deliver one. Neither neighbour works either: `P - 1` has no room when the
//! client already holds `P - 1`, and `P + 1` is unallocated, so the real change
//! that later takes it is dropped in silence. That case sends a snapshot.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};
use tracing::{debug, warn};

use super::dao::ConversationDao;
use super::{AgentModelDefaults, projection};
use crate::data::T3WorkspaceDao;
use crate::listing::{UiRpcChangeSubject, UiRpcEnvironmentIdentity};
use moatless_backend_core::activity::{self, ConversationChange, ConversationHead};
use moatless_backend_core::scope::ScopeContext;

/// How many recorded changes one poll reads. A cap rather than a limit — the
/// cursor still advances to the last record read, so a full batch only splits
/// the delivery in two. Saturation is logged because a poll that is always full
/// means the interval is too long for the deployment's change rate.
const CHANGES_PER_POLL: i64 = 500;

/// A client's place in the change feed, the database that place is in, and what
/// the client is holding.
///
/// Owned by one subscription. The cursor moves over *records*, not delivered
/// events, so a change this viewer cannot see advances it and sends nothing; the
/// identity travels with it because a cursor without one is a number naming a
/// different change in every database. All four move together or not at all.
///
/// # Why `grouped` is not the same set as `projects`
///
/// Two questions, and one set cannot answer both (Decision 11). *May this id be
/// taken back?* Only ids a delivery put in `threads`/`projects`, or a removal
/// names a row this client was never shown. *Can the client group this row?* It
/// drops one whose project it has no group for, in silence, and guessing wrong
/// there costs a snapshot rather than a disclosure. A resume can answer the
/// second soundly and not the first, so `grouped` is seeded separately in
/// [`shell_subscription`](super::shell_subscription). `projects ⊆ grouped`.
pub struct ShellFeed {
    identity: UiRpcEnvironmentIdentity,
    cursor: i64,
    threads: HashSet<String>,
    projects: HashSet<String>,
    grouped: HashSet<String>,
}

impl ShellFeed {
    /// Start from the position a snapshot was read at, in the database it was
    /// read from, holding nothing.
    ///
    /// The removable sets stay empty for the life of a resumed feed, and
    /// deliberately: reconstructing them from what the viewer can read now is
    /// wrong in the one direction that matters, since the rows missing are
    /// exactly the ones that left while the client was away (Decision 11).
    /// [`ShellFeed::grouping_by`] keeps that from also costing a snapshot per
    /// reconnection; callers holding the snapshot use [`ShellFeed::holding`].
    pub fn resuming_from(identity: UiRpcEnvironmentIdentity, position: i64) -> Self {
        Self {
            identity,
            cursor: position,
            threads: HashSet::new(),
            projects: HashSet::new(),
            grouped: HashSet::new(),
        }
    }

    /// Seed what the client holds, from the listing it was sent — both of the
    /// struct's questions at once, which only a caller holding the snapshot can
    /// answer.
    pub fn holding(mut self, threads: Vec<String>, projects: Vec<String>) -> Self {
        self.threads = threads.into_iter().collect();
        self.projects = projects.into_iter().collect();
        self.grouped = self.projects.clone();
        self
    }

    /// Seed the projects the client can *group* by, without claiming this feed
    /// sent them — the resume path's half of [`ShellFeed::holding`], and
    /// deliberately the weaker half. These ids gate delta delivery and never
    /// gate a removal, so a wrong one costs a row not rendered until the next
    /// rebuild rather than a removal naming something never shown.
    pub fn grouping_by(mut self, projects: Vec<String>) -> Self {
        self.grouped = projects.into_iter().collect();
        self.grouped.extend(self.projects.iter().cloned());
        self
    }

    /// Everything this viewer should be told since the last poll, oldest first.
    /// Empty means nothing changed, or nothing that changed is theirs.
    ///
    /// The head is read as well as the changes because a subscription outliving
    /// the database it opened against never resumes, so the guards never run on
    /// it: the poll asks for changes past a position the replacement has not
    /// reached — silence, then changes, having skipped everything between.
    /// Decision 7's third place the check has to run.
    pub async fn poll(
        &mut self,
        workspaces: &dyn T3WorkspaceDao,
        conversations: &dyn ConversationDao,
        defaults: &AgentModelDefaults,
        viewer: &ScopeContext,
    ) -> Result<Vec<Value>, sqlx::Error> {
        let head = conversations.listing_head().await?;
        if head.identity != self.identity {
            return self
                .rebuild(workspaces, conversations, defaults, viewer)
                .await;
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

        // One delivery per subject, at the highest position its burst reached,
        // re-sorted by the position each survivor kept.
        let mut latest: HashMap<(bool, &str), i64> = HashMap::new();
        for change in &changes {
            let key = match change.subject() {
                Some(UiRpcChangeSubject::Task(task_id)) => (false, task_id),
                Some(UiRpcChangeSubject::Workspace(workspace_id)) => (true, workspace_id),
                None => {
                    warn!(
                        position = change.position,
                        "Listing change names neither a task nor a workspace; skipping it"
                    );
                    continue;
                }
            };
            latest
                .entry(key)
                .and_modify(|position| *position = (*position).max(change.position))
                .or_insert(change.position);
        }
        let mut coalesced: Vec<((bool, &str), i64)> = latest.into_iter().collect();
        coalesced.sort_by_key(|((is_workspace, id), position)| (*position, *is_workspace, *id));
        if coalesced.is_empty() {
            // Every record named neither subject — unreachable under the CHECK
            // and already warned about, so the cursor moves past them.
            self.cursor = batch_end;
            return Ok(Vec::new());
        }

        let task_ids: Vec<String> = coalesced
            .iter()
            .filter(|((is_workspace, _), _)| !is_workspace)
            .map(|((_, id), _)| (*id).to_string())
            .collect();
        let rows = conversations
            .list_followed_by_ids(viewer, &task_ids)
            .await?;

        // The ones these conversations reference, plus changed workspaces this
        // client already holds — the second half lets a viewer who cannot *read*
        // one still be told it was renamed, disclosing no id they lack.
        let mut wanted: Vec<String> = rows
            .iter()
            .filter_map(|row| row.workspace_id.clone())
            .collect();
        wanted.extend(
            coalesced
                .iter()
                .filter(|((is_workspace, id), _)| *is_workspace && self.projects.contains(*id))
                .map(|((_, id), _)| (*id).to_string()),
        );
        let projects = workspaces.list_readable_including(viewer, &wanted).await?;

        let by_id: HashMap<&str, &_> = rows.iter().map(|row| (row.task_id.as_str(), row)).collect();
        let mut items = Vec::new();
        let mut delivered_threads: Vec<&str> = Vec::new();
        let mut delivered_projects: Vec<&str> = Vec::new();
        let mut dropped_threads: Vec<&str> = Vec::new();
        let mut dropped_projects: Vec<&str> = Vec::new();

        for ((is_workspace, id), position) in coalesced {
            if is_workspace {
                match projects.iter().find(|project| project.workspace.id == id) {
                    Some(project) => {
                        items.push(projection::project_upserted(project, position));
                        delivered_projects.push(id);
                    }
                    // Deleted, or no longer readable and no longer referenced.
                    // Only a project this client was sent can be taken back.
                    None if self.projects.contains(id) => {
                        items.push(projection::project_removed(id, position));
                        dropped_projects.push(id);
                    }
                    None => {}
                }
                continue;
            }

            let row = by_id.get(id);
            let project_id = row.and_then(|row| {
                row.workspace_id
                    .as_deref()
                    .filter(|id| projects.iter().any(|project| project.workspace.id == *id))
            });
            match (row, project_id) {
                (Some(row), Some(project_id)) => {
                    if !self.grouped.contains(project_id)
                        && !delivered_projects.contains(&project_id)
                    {
                        // Entered a project this client has no group for, and
                        // the pair cannot go out at one position.
                        return self
                            .resynchronize(workspaces, conversations, defaults, viewer)
                            .await;
                    }
                    items.push(projection::thread_upserted(
                        row, project_id, defaults, position,
                    ));
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
                        items.push(projection::thread_removed(id, position));
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
                        items.push(projection::thread_removed(id, position));
                        dropped_threads.push(id);
                    }
                }
            }
        }

        for id in delivered_threads {
            self.threads.insert(id.to_string());
        }
        for id in delivered_projects {
            self.projects.insert(id.to_string());
            self.grouped.insert(id.to_string());
        }
        for id in dropped_threads {
            self.threads.remove(id);
        }
        for id in dropped_projects {
            self.projects.remove(id);
            self.grouped.remove(id);
        }
        // Last, with the items built and nothing fallible left between here and
        // the caller. See `batch_end`.
        self.cursor = batch_end;
        Ok(items)
    }

    /// Put a watching client back onto a listing built from the database that is
    /// actually there now.
    ///
    /// The client *replaces* the snapshot it holds rather than folding into it
    /// (`client-runtime/src/state/shell.ts:141-147`), so nothing has to be told
    /// which rows went, and the feed continues from the new identity as well as
    /// the new position.
    ///
    /// The cursor is *set* here, not advanced — the one thing this does not
    /// share with [`ShellFeed::resynchronize`]. The old cursor came from a
    /// counter that is gone, so a `max()` of the two would park the feed above a
    /// position the replacement has not reached and answer every poll with
    /// silence until it got there.
    async fn rebuild(
        &mut self,
        workspaces: &dyn T3WorkspaceDao,
        conversations: &dyn ConversationDao,
        defaults: &AgentModelDefaults,
        viewer: &ScopeContext,
    ) -> Result<Vec<Value>, sqlx::Error> {
        let snapshot = super::shell_snapshot(workspaces, conversations, defaults, viewer).await?;
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
        workspaces: &dyn T3WorkspaceDao,
        conversations: &dyn ConversationDao,
        defaults: &AgentModelDefaults,
        viewer: &ScopeContext,
    ) -> Result<Vec<Value>, sqlx::Error> {
        let snapshot = super::shell_snapshot(workspaces, conversations, defaults, viewer).await?;
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

    /// Take a freshly built listing as the client's whole state, and hand it
    /// over.
    ///
    /// Both callers replace what the client is looking at, so both replace
    /// *everything this feed believes about it* — the id sets included. Leaving
    /// those is silent both ways: a row the snapshot introduced is in no set and
    /// is never removed, and one from the listing it replaced is still in one,
    /// so a removal names a row the client does not have.
    fn adopt(&mut self, snapshot: super::ShellSnapshot, cursor: i64) -> Vec<Value> {
        self.threads = snapshot.thread_ids().into_iter().collect();
        self.projects = snapshot.project_ids().into_iter().collect();
        // The snapshot answers both questions, so a resuming feed stops guessing.
        self.grouped = self.projects.clone();
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
/// The same mechanism as the shell's and against a different log: every write
/// to a Message a conversation shows records a position against that
/// conversation (`task_activities`), and this reads forward from the position
/// the client holds. What it sends is the contract's own events, so a change is
/// the few hundred bytes of the row that moved rather than the whole transcript.
///
/// # Coalescing
///
/// One Message is assembled from its blocks by a run of in-place merges, so a
/// single assistant turn records a change per block and each of them names the
/// same Message. Re-reading that Message once per change would send the same
/// bubble several times over, so a poll keeps only the last position each
/// Message was named at. The events are the same either way — they are upserts
/// — but the last position is the one that carries the whole row.
///
/// The poll interval is `THREAD_POLL_INTERVAL` in the crate root.
pub struct ThreadFeed {
    task_id: String,
    /// The position this feed reads from next, in the change log's own
    /// numbering rather than the contract's.
    cursor: i64,
    /// The sequence the client has been sent, below which an event of a
    /// re-read change is dropped rather than sent again. See [`ThreadFeed::poll`].
    delivered: i64,
}

/// How many recorded changes one poll reads.
///
/// A cap rather than a window: what is left over is read by the next poll from
/// the cursor this one advanced to, so a client behind by more than this
/// catches up over several intervals instead of one large frame. Four times the
/// shell's, because a conversation records a position per Message block and the
/// shell records one per Task.
const THREAD_POLL_CHANGES: i64 = 512;

impl ThreadFeed {
    /// Start from the position a snapshot was built at. The client holds
    /// everything that position covers, so the first live poll is looking for
    /// what came after it.
    pub fn at(task_id: String, head: ConversationHead) -> Self {
        Self {
            task_id,
            cursor: head.position + 1,
            delivered: head.sequence(),
        }
    }

    /// Start from the sequence a returning client says it holds.
    ///
    /// The cursor is the position that sequence sits *inside*, not the one
    /// after it: a client disconnected partway through one change's events
    /// holds a sequence in the middle of that change, and the events it missed
    /// are the rest of that same change. Re-reading the whole change and
    /// dropping the events at or below `after` is what makes the boundary
    /// exact — see [`activity::position_of`].
    pub fn resuming_from(task_id: String, after: i64) -> Self {
        Self {
            task_id,
            cursor: activity::position_of(after),
            delivered: after,
        }
    }

    /// Everything recorded since this feed last read, as stream items — an
    /// empty list when nothing was, and `None` when the conversation is gone.
    ///
    /// "Gone" is deliberately one answer for three different endings — deleted,
    /// unfollowed, or no longer readable — because they are the same fact to
    /// the client and separating them here would tell it which. The
    /// subscription ends; the *shell* feed is what tells a client a
    /// conversation left its sidebar, and it applies the disclosure rule that
    /// decision needs.
    ///
    /// The changes are read before the rows they name, and the rows are read
    /// once for the whole batch. So a row that moves again between the two
    /// reads is projected in its newer state under the older change's
    /// sequence — which is why the next poll re-reads it: the newer change has
    /// its own position, still ahead of this cursor.
    ///
    /// Every read here happens before the cursor moves, so a poll that fails
    /// part-way leaves this feed exactly where it started and the next one
    /// covers the same ground. That is the invariant `watch_thread` survives a
    /// transient database error on.
    ///
    /// Neither the workspace listing nor the model defaults are read: a thread
    /// event carries no grouping and no model, unlike the shell rows and the
    /// snapshot this feed continues from.
    pub async fn poll(
        &mut self,
        conversations: &dyn ConversationDao,
        viewer: &ScopeContext,
        frontend_url: &str,
    ) -> Result<Option<Vec<Value>>, sqlx::Error> {
        let changes = conversations
            .conversation_changes(&self.task_id, self.cursor, THREAD_POLL_CHANGES)
            .await?;

        // The conversation itself, through the same narrowed listing the
        // snapshot resolved it with — which is where this poll's access check
        // lives, and why a conversation that left this viewer's reach needs no
        // rule repeated here.
        //
        // Read on every poll, including the silent ones. Closing, deleting or
        // unfollowing a Task records no *conversation* change — nothing about
        // its Messages moved — so a check that ran only when something was
        // recorded would never run at all on the conversation most likely to
        // have ended, and its subscription would sit open on a thread that is
        // gone.
        let rows = conversations
            .list_followed_by_ids(viewer, std::slice::from_ref(&self.task_id))
            .await?;
        let Some(conversation) = rows.into_iter().next() else {
            debug!(
                task_id = %self.task_id,
                user_id = %viewer.user_id,
                "The conversation a T3 UI thread subscription watches is no longer visible to it"
            );
            return Ok(None);
        };

        let Some(last) = changes.last() else {
            return Ok(Some(Vec::new()));
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

        let mut items = Vec::new();
        for change in &changes {
            for item in projection::change_events(&conversation, change, &window, &origins) {
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

/// One change per named row, keeping the last position each was named at, in
/// position order. A row is named by a Message uuid or, for the activities that
/// happened on no Message, by its own activity id; both fold the same way.
/// Changes naming neither are the conversation's own and are all kept: each is a
/// distinct transition of its session.
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
    use crate::dao::{Conversation, MockConversationDao, Transcript, TranscriptMessage};
    use moatless_api_schemas::enums::SandboxDesiredState;
    use moatless_api_schemas::enums::{MessageRole, MessageType};
    use moatless_api_schemas::shared::AgentType;
    use moatless_backend_core::activity::EVENTS_PER_CHANGE;

    fn change(position: i64, uuid: Option<&str>) -> ConversationChange {
        ConversationChange {
            position,
            message_uuid: uuid.map(str::to_string),
            activity_id: None,
        }
    }

    fn activity_change(position: i64, activity_id: &str) -> ConversationChange {
        ConversationChange {
            position,
            message_uuid: None,
            activity_id: Some(activity_id.to_string()),
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
        let feed = ThreadFeed::at("task-a".into(), ConversationHead { position: 7 });
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
            agent_type: AgentType(AgentType::CLAUDE_CODE.to_string()),
            created_at: "2026-07-01T09:00:00.000Z".into(),
            created_at_db: None,
            updated_at: None,
            closed_at: None,
            turn_number: Some(3),
            turn_status: Some("active".into()),
            turn_started_at: None,
            turn_ended_at: None,
            sandbox_desired_state: Some(SandboxDesiredState::Running),
            sandbox_stop_reason: None,
            sandbox_started_at: None,
            sandbox_updated_at: None,
            latest_user_message_at: None,
            assistant_model: None,
            requested_model: None,
            pending_question: false,
            pending_plan: false,
            settled_override: None,
            settled_at: None,
            snoozed_until: None,
            snoozed_at: None,
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
        }
    }

    /// One change naming one Message, over a DAO that answers the three reads a
    /// poll makes.
    fn a_dao(changes: Vec<ConversationChange>) -> MockConversationDao {
        let mut dao = MockConversationDao::new();
        dao.expect_conversation_changes()
            .returning(move |_, _, _| Ok(changes.clone()));
        dao.expect_list_followed_by_ids()
            .returning(|_, _| Ok(vec![a_conversation()]));
        dao.expect_changed_messages().returning(|_, _, _| {
            Ok(Transcript {
                messages: vec![a_message(11, "msg-a")],
                activities: Vec::new(),
                omitted: 0,
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
        let mut feed = ThreadFeed::at("task-a".into(), ConversationHead { position: 8 });
        let items = feed.poll(&dao, &viewer(), "http://localhost:3000").await.unwrap().unwrap();

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

    /// The client drops an event at or below the position it holds, in silence.
    /// A resuming feed re-reads the change its cursor sits inside, so it is this
    /// side that has to drop the events the client already has.
    #[tokio::test]
    async fn a_resuming_poll_drops_the_events_the_client_already_holds() {
        let dao = a_dao(vec![change(9, Some("msg-a"))]);
        let already = 9 * EVENTS_PER_CHANGE;
        let mut feed = ThreadFeed::resuming_from("task-a".into(), already);
        let items = feed.poll(&dao, &viewer(), "http://localhost:3000").await.unwrap().unwrap();
        assert!(
            items.is_empty(),
            "nothing above the client's position: {items:#?}"
        );
        assert_eq!(feed.delivered, already);
    }

    /// A conversation that left this viewer's reach ends the subscription —
    /// one answer for deleted, unfollowed and no longer readable.
    #[tokio::test]
    async fn a_poll_that_can_no_longer_see_the_conversation_ends_the_subscription() {
        let mut dao = MockConversationDao::new();
        dao.expect_conversation_changes()
            .returning(|_, _, _| Ok(vec![change(9, Some("msg-a"))]));
        dao.expect_list_followed_by_ids()
            .returning(|_, _| Ok(Vec::new()));
        let mut feed = ThreadFeed::at("task-a".into(), ConversationHead { position: 8 });
        assert!(feed.poll(&dao, &viewer(), "http://localhost:3000").await.unwrap().is_none());
    }

    /// An idle conversation is not a gone one, and a subscription that ended on
    /// silence would end every one of them.
    #[tokio::test]
    async fn a_poll_over_a_conversation_that_did_not_move_says_nothing() {
        let mut dao = MockConversationDao::new();
        dao.expect_conversation_changes()
            .returning(|_, _, _| Ok(Vec::new()));
        dao.expect_list_followed_by_ids()
            .returning(|_, _| Ok(vec![a_conversation()]));
        let mut feed = ThreadFeed::at("task-a".into(), ConversationHead { position: 8 });
        assert_eq!(feed.poll(&dao, &viewer(), "http://localhost:3000").await.unwrap(), Some(Vec::new()));
        assert_eq!(feed.cursor, 9, "and the cursor stayed where it was");
    }

    /// The end a conversation is most likely to reach is the one that records
    /// no change: closing, deleting or unfollowing a Task moves none of its
    /// Messages. A check that ran only when something was recorded would never
    /// run on those, and the subscription would sit open on a thread that is
    /// gone — which is why the silent poll above still reads the row.
    #[tokio::test]
    async fn an_idle_subscription_still_discovers_that_its_conversation_ended() {
        let mut dao = MockConversationDao::new();
        dao.expect_conversation_changes()
            .returning(|_, _, _| Ok(Vec::new()));
        dao.expect_list_followed_by_ids()
            .returning(|_, _| Ok(Vec::new()));
        let mut feed = ThreadFeed::at("task-a".into(), ConversationHead { position: 8 });
        assert!(
            feed.poll(&dao, &viewer(), "http://localhost:3000").await.unwrap().is_none(),
            "a conversation that ended without recording a change still ends the subscription"
        );
    }

    /// `watch_thread` keeps a subscription alive across a failed poll on the
    /// ground that the cursor has not moved, so the next poll covers the same
    /// ground. A cursor advanced before the reads that can fail makes that
    /// sentence false and loses the batch outright: nothing logged on the
    /// client's side, nothing wrong on its screen except that it is frozen.
    #[tokio::test]
    async fn a_poll_that_fails_part_way_leaves_the_cursor_where_it_started() {
        let mut dao = MockConversationDao::new();
        dao.expect_conversation_changes()
            .returning(|_, _, _| Ok(vec![change(9, Some("msg-a"))]));
        dao.expect_list_followed_by_ids()
            .returning(|_, _| Ok(vec![a_conversation()]));
        // The read that comes after the cursor would have moved.
        dao.expect_changed_messages()
            .returning(|_, _, _| Err(sqlx::Error::PoolTimedOut));

        let mut feed = ThreadFeed::at("task-a".into(), ConversationHead { position: 8 });
        assert!(feed.poll(&dao, &viewer(), "http://localhost:3000").await.is_err());
        assert_eq!(
            feed.cursor, 9,
            "the next poll has to read change 9 again, or it is never sent"
        );
    }
}
