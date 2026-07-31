//! Turning recorded listing changes into the events a watching client applies.
//!
//! The client folds events into the snapshot it holds and discards anything at
//! or below the position it already has, so this side never sends a difference:
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

use serde_json::{json, Value};
use tracing::{debug, warn};

use super::dao::{ConversationDao, ThreadRevision};
use super::{projection, AgentModelDefaults};
use crate::task_listing::{ChangeSubject, EnvironmentIdentity};
use crate::workspace::dao::WorkspaceDao;
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
    identity: EnvironmentIdentity,
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
    pub fn resuming_from(identity: EnvironmentIdentity, position: i64) -> Self {
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
        workspaces: &dyn WorkspaceDao,
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
                Some(ChangeSubject::Task(task_id)) => (false, task_id),
                Some(ChangeSubject::Workspace(workspace_id)) => (true, workspace_id),
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
        let mut wanted: Vec<String> = rows.iter().filter_map(|row| row.workspace_id.clone()).collect();
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
                match projects
                    .iter()
                    .find(|project| project.workspace.id == id)
                {
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
                    items.push(projection::thread_upserted(row, project_id, defaults, position));
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
                // client holds, never from the empty re-read.
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
        workspaces: &dyn WorkspaceDao,
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
        workspaces: &dyn WorkspaceDao,
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
/// Deliberately not the shell's mechanism. The shell's feed reads a recorded
/// change log and sends a difference; a conversation has no such log — only
/// turn transitions are recorded there, so an agent writing message after
/// message inside one turn records nothing, and a feed built on it would leave
/// a running conversation frozen until the turn ended.
///
/// So this compares a fingerprint instead, and sends the whole conversation
/// whenever it moved. The client replaces what it holds on every snapshot
/// (`state/threads.ts:186-190`), so a resend is complete rather than additive
/// and nothing has to reconcile.
///
/// **The cost is the resend.** One open conversation re-crosses the socket
/// whole on each change, which is why [`THREAD_POLL_INTERVAL`] is four times
/// the shell's and why the transcript is capped. Replacing this with the
/// contract's `thread.message-sent` / `thread.activity-appended` events is what
/// removes the cost, and needs an event log this backend does not yet keep.
///
/// [`THREAD_POLL_INTERVAL`]: super::THREAD_POLL_INTERVAL
pub struct ThreadFeed {
    task_id: String,
    /// What the conversation looked like when the client was last sent it.
    /// `None` only before the first poll of a conversation that had no rows.
    revision: Option<ThreadRevision>,
}

impl ThreadFeed {
    /// Start from the fingerprint a snapshot was built at.
    pub fn at(task_id: String, revision: Option<ThreadRevision>) -> Self {
        Self { task_id, revision }
    }

    /// The conversation again if it moved, nothing if it did not, and `None`
    /// when it is gone.
    ///
    /// "Gone" is deliberately one answer for three different endings — deleted,
    /// unfollowed, or no longer readable — because they are the same fact to
    /// the client and separating them here would tell it which. The
    /// subscription ends; the *shell* feed is what tells a client a
    /// conversation left its sidebar, and it applies the disclosure rule that
    /// decision needs.
    ///
    /// The fingerprint is re-read before the conversation, never after: taken
    /// afterwards it would describe a state the sent snapshot already includes,
    /// and the next change would compare equal and never be sent — one
    /// conversation frozen, silently. Read first, the worst case is one extra
    /// resend of a snapshot that is identical to the last.
    pub async fn poll(
        &mut self,
        workspaces: &dyn WorkspaceDao,
        conversations: &dyn ConversationDao,
        defaults: &AgentModelDefaults,
        viewer: &ScopeContext,
    ) -> Result<Option<Vec<Value>>, sqlx::Error> {
        let revision = conversations.thread_revision(&self.task_id).await?;
        let Some(revision) = revision else {
            debug!(
                task_id = %self.task_id,
                user_id = %viewer.user_id,
                "The conversation a T3 UI thread subscription watches no longer exists"
            );
            return Ok(None);
        };
        if self.revision.as_ref() == Some(&revision) {
            return Ok(Some(Vec::new()));
        }
        self.revision = Some(revision);

        let snapshot =
            super::thread_snapshot(workspaces, conversations, defaults, viewer, &self.task_id)
                .await?;
        let Some(snapshot) = snapshot else {
            debug!(
                task_id = %self.task_id,
                user_id = %viewer.user_id,
                "The conversation a T3 UI thread subscription watches is no longer visible to it"
            );
            return Ok(None);
        };
        Ok(Some(vec![json!({
            "kind": "snapshot",
            "snapshot": snapshot.value
        })]))
    }
}
