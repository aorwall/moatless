//! Replaying sources into a conversation: a fork's inheritance at creation,
//! and `rebuild`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;
use sqlx::PgConnection;
use uuid::Uuid;

use moatless_core::change_feed::conversation::{record_activity_change, record_message_change};

use super::dao::FrozenCopy;
use super::ids;
use super::projector::{EventSource, Line, Projector};
use super::replay_dao::{self, ReplayEvent, ReplayLine};
use crate::inbox::InboxItem;
use crate::trajectory::models::TrajectorySession;

/// Source rows per `rebuild` transaction.
const REBUILD_PAGE: usize = 1_000;

enum Step {
    Item(Box<InboxItem>),
    Line(ReplayLine),
    Event(ReplayEvent),
    /// A frozen ancestor's history through its cut, which no source replays.
    Frozen {
        ancestor: String,
        turn: Option<i32>,
        cut_at: DateTime<Utc>,
    },
}

impl Step {
    fn at(&self) -> DateTime<Utc> {
        match self {
            Self::Item(item) => item.created_at,
            Self::Line(line) => line.created_at,
            Self::Event(event) => event.created_at,
            Self::Frozen { .. } => DateTime::<Utc>::MIN_UTC,
        }
    }
}

impl Projector {
    /// Projects a fork's inherited conversation from each ancestor's sources up
    /// to its cut (4.9a), a frozen ancestor's copied rows before them, and
    /// writes the fork's closed cut turn. Runs before
    /// the fork's Task row exists, so it records no listing change.
    pub(crate) async fn inherit(
        &self,
        tx: &mut PgConnection,
        fork_task_id: &str,
        source_task_id: &str,
        cut: Uuid,
    ) -> Result<(), sqlx::Error> {
        let lines = replay_dao::lineage_lines(&mut *tx, cut).await?;
        let turn = lines
            .iter()
            .find(|line| line.id == cut)
            .and_then(|line| line.turn_number);
        let sessions = load_sessions(&mut *tx, &lines).await?;
        let steps = ancestor_steps(&mut *tx, fork_task_id, lines).await?;
        for step in &steps {
            self.replay(&mut *tx, fork_task_id, step, &sessions).await?;
        }
        if let Some(turn) = turn {
            replay_dao::close_cut_turn(&mut *tx, fork_task_id, source_task_id, turn).await?;
        }
        Ok(())
    }

    /// Deletes `task_id`'s non-frozen rows and replays its sources, an
    /// ancestor's first, in pages of [`REBUILD_PAGE`]. Positions are
    /// reassigned; a revert change to turn 0 tells a reader to start over.
    pub(crate) async fn rebuild(&self, pool: &DbPool, task_id: &str) -> Result<(), sqlx::Error> {
        let mut tx = pool.begin().await?;
        let frozen_at = replay_dao::frozen_at(&mut tx, task_id).await?;
        let lines = match replay_dao::conversation_tip(&mut tx, task_id).await? {
            Some(tip) => replay_dao::lineage_lines(&mut tx, tip).await?,
            None => Vec::new(),
        };
        let sessions = load_sessions(&mut tx, &lines).await?;
        let (own, inherited): (Vec<_>, Vec<_>) =
            lines.into_iter().partition(|line| line.task_id == task_id);
        let mut steps = ancestor_steps(&mut tx, task_id, inherited).await?;

        let mut own_steps: Vec<Step> = own
            .into_iter()
            .filter(|line| frozen_at.is_none_or(|frozen| line.created_at > frozen))
            .map(Step::Line)
            .collect();
        own_steps.extend(
            replay_dao::inbox_items(&mut tx, task_id, None, frozen_at)
                .await?
                .into_iter()
                .map(|item| Step::Item(Box::new(item))),
        );
        own_steps.extend(
            replay_dao::task_events(&mut tx, task_id, None, frozen_at)
                .await?
                .into_iter()
                .map(Step::Event),
        );
        own_steps.sort_by_key(Step::at);
        steps.extend(own_steps);

        super::dao::delete_projected(&mut tx, task_id, None).await?;
        moatless_core::change_feed::conversation::record_revert_change(&mut tx, task_id, 0).await?;
        tx.commit().await?;

        for page in steps.chunks(REBUILD_PAGE) {
            let mut tx = pool.begin().await?;
            for step in page {
                self.replay(&mut tx, task_id, step, &sessions).await?;
            }
            tx.commit().await?;
        }
        let mut tx = pool.begin().await?;
        moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
        tx.commit().await
    }

    async fn replay(
        &self,
        tx: &mut PgConnection,
        task_id: &str,
        step: &Step,
        sessions: &HashMap<Uuid, TrajectorySession>,
    ) -> Result<(), sqlx::Error> {
        match step {
            Step::Item(item) => self.project_item(tx, task_id, item, false).await,
            Step::Line(line) => {
                let Some(session) = sessions.get(&line.session_id) else {
                    return Ok(());
                };
                let projected = Line {
                    id: line.id,
                    seq: line.seq,
                    harness_item_id: line.harness_item_id.as_deref(),
                    content: &line.content,
                    turn_number: line.turn_number,
                };
                self.project_lines(tx, task_id, session, &[projected], false)
                    .await
                    .map(|_| ())
            }
            Step::Event(event) => {
                let source = EventSource {
                    event_id: &event.event_id,
                    event_type: &event.event_type,
                    sub_type: &event.sub_type,
                    is_error: event.is_error,
                    turn_number: event.turn_number,
                    payload: &event.payload,
                };
                self.project_event(tx, task_id, &source).await
            }
            Step::Frozen {
                ancestor,
                turn,
                cut_at,
            } => copy_frozen(tx, task_id, ancestor, *turn, *cut_at).await,
        }
    }
}

/// Copies `ancestor`'s frozen rows through the cut onto `task_id`, each top
/// level one at a new position in their order. A fork that holds them, as a
/// rebuild keeps them, copies none.
async fn copy_frozen(
    tx: &mut PgConnection,
    task_id: &str,
    ancestor: &str,
    turn: Option<i32>,
    cut_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    if super::dao::holds_frozen(&mut *tx, task_id).await? {
        return Ok(());
    }
    let message_id = |from: Uuid| ids::message(task_id, &format!("frozen:{from}"));
    let mut messages = Vec::new();
    let mut activities = Vec::new();
    for row in replay_dao::frozen_rows(&mut *tx, ancestor, turn, cut_at).await? {
        let to = if row.message {
            message_id(row.id)
        } else {
            ids::activity(&ids::scope(task_id, &format!("frozen:{}", row.id)))
        };
        let position = match (&row.parent_tool_use_id, row.message) {
            (Some(_), _) => row.position,
            (None, true) => record_message_change(&mut *tx, task_id, &to.to_string()).await?,
            (None, false) => record_activity_change(&mut *tx, task_id, &to.to_string()).await?,
        };
        let copy = FrozenCopy {
            from: row.id,
            to,
            position,
            message: row.message_uuid.map(message_id),
        };
        if row.message {
            messages.push(copy);
        } else {
            activities.push(copy);
        }
    }
    let usage: Vec<(Uuid, Uuid)> = messages
        .iter()
        .map(|copy| copy.from)
        .chain(replay_dao::orphan_frozen_usage(&mut *tx, ancestor).await?)
        .map(|from| (from, message_id(from)))
        .collect();
    super::dao::copy_frozen(tx, task_id, ancestor, &messages, &activities, &usage).await
}

async fn load_sessions(
    tx: &mut PgConnection,
    lines: &[ReplayLine],
) -> Result<HashMap<Uuid, TrajectorySession>, sqlx::Error> {
    let mut ids: Vec<Uuid> = lines.iter().map(|line| line.session_id).collect();
    ids.sort_unstable();
    ids.dedup();
    Ok(replay_dao::sessions(tx, &ids)
        .await?
        .into_iter()
        .map(|session| (session.id, session))
        .collect())
}

/// Each ancestor's steps up to its cut, oldest ancestor first; `lines` are
/// the lineage lines the ancestors' sessions hold, in `ord` order. The cut in
/// an ancestor is its last lineage line, and the cut turn that line's turn.
async fn ancestor_steps(
    tx: &mut PgConnection,
    task_id: &str,
    lines: Vec<ReplayLine>,
) -> Result<Vec<Step>, sqlx::Error> {
    let mut segments: Vec<(String, Vec<ReplayLine>)> = Vec::new();
    for line in lines {
        if line.task_id == task_id {
            continue;
        }
        match segments.iter_mut().find(|(task, _)| *task == line.task_id) {
            Some((_, held)) => held.push(line),
            None => segments.push((line.task_id.clone(), vec![line])),
        }
    }
    let mut steps = Vec::new();
    for (ancestor, lines) in segments {
        let frozen_at = replay_dao::frozen_at(&mut *tx, &ancestor).await?;
        let cut = lines.iter().rev().find(|line| !line.subagent);
        let cut_at = cut.map_or_else(Utc::now, |line| line.created_at);
        let turn = cut.and_then(|line| line.turn_number).or_else(|| {
            lines
                .iter()
                .rev()
                .find_map(|line| line.turn_number.filter(|_| !line.subagent))
        });
        // A frozen ancestor's sources up to the freeze are its frozen rows.
        let mut segment: Vec<Step> = lines
            .into_iter()
            .filter(|line| frozen_at.is_none_or(|frozen| line.created_at > frozen))
            .map(Step::Line)
            .collect();
        if frozen_at.is_some() {
            segment.push(Step::Frozen {
                ancestor: ancestor.clone(),
                turn,
                cut_at,
            });
        }
        if let Some(turn) = turn {
            segment.extend(
                replay_dao::inbox_items(&mut *tx, &ancestor, Some((turn, cut_at)), frozen_at)
                    .await?
                    .into_iter()
                    .map(|item| Step::Item(Box::new(item))),
            );
        }
        segment.extend(
            replay_dao::task_events(
                &mut *tx,
                &ancestor,
                Some((turn.unwrap_or(i32::MIN), cut_at)),
                frozen_at,
            )
            .await?
            .into_iter()
            .map(Step::Event),
        );
        segment.sort_by_key(Step::at);
        steps.extend(segment);
    }
    Ok(steps)
}
