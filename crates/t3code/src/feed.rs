//! Turning the sidebar listing's recorded changes into the events a watching//! client applies.
//!
//! The client discards anything at or below the position it already holds, so
//! a cursor is handed back only in the numbering that allocated it. Removals
//! come from what the viewer previously held, never an empty re-read, which
//! would disclose who left; two events needed at one position are sent as a
//! snapshot instead.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};
use tracing::{debug, warn};

use super::{AgentModelDefaults, projection};
use crate::dao::ListingError;
use crate::reads::T3Reads;
use moatless_core::change_feed::listing::{EnvironmentIdentity, ListingHead, ListingSubject};
use moatless_types::access_control::ScopeContext;

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
    identity: EnvironmentIdentity,
    cursor: i64,
    threads: HashSet<String>,
    grouped: HashSet<String>,
}

impl ShellFeed {
    /// Start from the position a snapshot was read at, in that database,
    /// holding nothing.
    ///
    /// The removable set stays empty deliberately: a fresh read would miss rows
    /// that left while the client was away.
    pub fn resuming_from(identity: EnvironmentIdentity, position: i64) -> Self {
        Self {
            identity,
            cursor: position,
            threads: HashSet::new(),
            grouped: HashSet::new(),
        }
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

    /// The listing head this feed has read through, in its database.
    pub(crate) fn read_through(&self) -> ListingHead {
        ListingHead {
            identity: self.identity.clone(),
            position: self.cursor,
        }
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
    ) -> Result<Vec<Value>, ListingError> {
        let head = reads.conversations().listing_head().await?;
        self.poll_at(&head, reads, defaults, viewer).await
    }

    /// [`ShellFeed::poll`] against a `head` read by the caller, which may be
    /// older than the changes this reads past it.
    pub(crate) async fn poll_at(
        &mut self,
        head: &ListingHead,
        reads: &T3Reads,
        defaults: &AgentModelDefaults,
        viewer: &ScopeContext,
    ) -> Result<Vec<Value>, ListingError> {
        let conversations = reads.conversations();
        let workspaces = reads.workspaces();
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

        // One delivery per subject, at the highest position its burst reached,
        // re-sorted by the position each survivor kept.
        let mut latest: HashMap<&ListingSubject, i64> = HashMap::new();
        for change in &changes {
            latest
                .entry(&change.subject)
                .and_modify(|position| *position = (*position).max(change.position))
                .or_insert(change.position);
        }
        let mut coalesced: Vec<(&ListingSubject, i64)> = latest.into_iter().collect();
        coalesced.sort_by_key(|(subject, position)| (*position, *subject));

        let mut task_ids: Vec<String> = Vec::new();
        let mut changed_workspaces: Vec<String> = Vec::new();
        for (subject, _) in &coalesced {
            match subject {
                ListingSubject::Task(id) => task_ids.push(id.clone()),
                ListingSubject::Workspace(id) => changed_workspaces.push(id.clone()),
            }
        }
        let rows = conversations
            .list_followed_by_ids(viewer, &task_ids)
            .await?;

        // The Workspaces these changed Tasks reference, to project their rows
        // and decide whether the client can group them, plus the changed
        // Workspaces themselves, read whatever their state so the loop below
        // can tell a live readable one from one gone out of reach.
        let mut wanted: Vec<String> = rows
            .iter()
            .filter_map(|row| row.workspace_id.clone())
            .collect();
        wanted.extend(changed_workspaces);
        let projects = workspaces.list_readable_including(viewer, &wanted).await?;

        let by_id: HashMap<&str, &_> = rows.iter().map(|row| (row.task_id.as_str(), row)).collect();
        let mut items = Vec::new();
        let mut delivered_threads: Vec<&str> = Vec::new();
        let mut dropped_threads: Vec<&str> = Vec::new();
        let mut delivered_projects: HashSet<&str> = HashSet::new();

        for (subject, position) in coalesced {
            let id = match subject {
                ListingSubject::Workspace(workspace_id) => {
                    let listed = projects
                        .iter()
                        .find(|project| project.workspace.id == *workspace_id);
                    match listed {
                        Some(listed) if listed.readable && !listed.workspace.deleted => {
                            items.push(projection::project_updated_v2(listed, position));
                            delivered_projects.insert(workspace_id.as_str());
                        }
                        // Deleted, or out of reach, and possibly still held: only
                        // a snapshot knows whether a followed thread keeps it
                        // listed, so it decides rather than a `project.removed`.
                        _ if self.grouped.contains(workspace_id.as_str()) => {
                            return self.resynchronize(reads, defaults, viewer).await;
                        }
                        // Never this viewer's to see: nothing to say, and saying
                        // anything would disclose the id.
                        _ => {}
                    }
                    continue;
                }
                ListingSubject::Task(task_id) => task_id.as_str(),
            };
            let row = by_id.get(id);
            let project_id = row.and_then(|row| {
                row.workspace_id
                    .as_deref()
                    .filter(|id| projects.iter().any(|project| project.workspace.id == *id))
            });
            match (row, project_id) {
                (Some(row), Some(project_id)) => {
                    if !self.grouped.contains(project_id)
                        && !delivered_projects.contains(project_id)
                    {
                        // Entered a project this client has no group for, and
                        // the pair cannot go out at one position.
                        return self.resynchronize(reads, defaults, viewer).await;
                    }
                    items.push(projection::thread_updated_v2(
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
                        items.push(projection::thread_removed_v2(id, position));
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
                        items.push(projection::thread_removed_v2(id, position));
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
        for id in delivered_projects {
            self.grouped.insert(id.to_string());
        }
        // Last, with the items built and nothing fallible left between here and
        // the caller. See `batch_end`.
        self.cursor = batch_end;
        Ok(items)
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
    ) -> Result<Vec<Value>, ListingError> {
        let snapshot = crate::snapshots::shell_snapshot(reads, defaults, viewer).await?;
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
    ) -> Result<Vec<Value>, ListingError> {
        let snapshot = crate::snapshots::shell_snapshot(reads, defaults, viewer).await?;
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
