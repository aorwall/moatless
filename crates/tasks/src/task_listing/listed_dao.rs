//! The Task listing query: one statement per read, every narrowing a clause.

use std::time::{Duration, Instant};

use sqlx::postgres::PgArguments;
use sqlx::query::QueryAs;
use sqlx::{AssertSqlSafe, Postgres};
use tokio::sync::Semaphore;
use tracing::warn;

use moatless_core::db::DbPool;
use moatless_types::access_control::{ScopeContext, ScopedEntity};
use moatless_types::activity::{PLAN_TOOL, QUESTION_TOOL};
use moatless_types::pull_request_key::PULL_REQUEST_KEY_PATTERN;
use moatless_types::task::{TaskRelationType, TaskStatus};

use super::listed::{ListedTask, ListingCursor};
use super::operations::{ListingError, ListingFilter, status_list};

pub(crate) const FULL_LISTING_CONCURRENCY: usize = 1;
const FULL_LISTING_STATEMENT_TIMEOUT: &str = "5s";
/// How long a full listing queues for the replica's permit before it is
/// refused. Shorter than [`FULL_LISTING_STATEMENT_TIMEOUT`], so a listing stuck
/// in the database refuses its queue rather than holding it for the full five
/// seconds.
pub(crate) const FULL_LISTING_QUEUE_WAIT: Duration = Duration::from_secs(2);

/// A capped read: `cap` rows, newest first, after `before` when given.
pub(crate) struct Page<'a> {
    pub(crate) cap: i64,
    pub(crate) before: Option<&'a ListingCursor>,
}

/// How wide a listing read reaches: both variants share the same read rule
/// — `Readable` widens past the follow join only, never past access — so
/// follower state (`settled_*`, `snoozed_*`) stays absent for a viewer who does
/// not follow, which is what the outer join expresses.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reach {
    /// The sidebar listings: involvement only.
    Followed,
    /// Opening one by id: a link resolves whether or not it's followed;
    /// writing to it is what follows it, via `message::service`.
    Readable,
}
pub(crate) const LISTING_DURATION: &str = "moatless_ui_shell_listing_duration_seconds";
pub(crate) const LISTING_REJECTIONS: &str = "moatless_ui_shell_listing_rejections_total";

/// The unnarrowed [`read`], behind the replica's permit: refused with
/// [`ListingError::AtCapacity`] once another held it for
/// [`FULL_LISTING_QUEUE_WAIT`], and [`ListingError::TimedOut`] once it runs
/// past [`FULL_LISTING_STATEMENT_TIMEOUT`].
pub(crate) async fn list(
    pool: &DbPool,
    permits: &Semaphore,
    viewer: &ScopeContext,
    reach: Reach,
    statuses: &[TaskStatus],
    browse: Option<&ListingFilter>,
    page: Page<'_>,
) -> Result<Vec<ListedTask>, ListingError> {
    // A browse shares this permit deliberately: it too is an unbounded
    // scan with no follow join to narrow it, and a permit of its own would
    // let two such reads hit the pool at once.
    let kind = if browse.is_some() {
        "browse"
    } else {
        "listing"
    };
    // Queued, not refused at once: a reconnecting client asks for its shell
    // and its archive in the same instant. The wait holds no connection.
    let Ok(Ok(_permit)) = tokio::time::timeout(FULL_LISTING_QUEUE_WAIT, permits.acquire()).await
    else {
        metrics::counter!(LISTING_REJECTIONS, "reason" => "at_capacity", "kind" => kind)
            .increment(1);
        warn!(
            user_id = %viewer.user_id,
            kind,
            wait_secs = FULL_LISTING_QUEUE_WAIT.as_secs(),
            "T3 UI full listing rejected; another one held the permit for the whole wait"
        );
        return Err(ListingError::AtCapacity);
    };
    let started = Instant::now();
    let result = read(pool, viewer, reach, statuses, None, browse, Some(page)).await;
    let elapsed = started.elapsed();
    let timed_out = matches!(
        &result,
        Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("57014")
    );
    let outcome = match &result {
        Ok(_) => "ok",
        Err(_) if timed_out => {
            metrics::counter!(
                LISTING_REJECTIONS,
                "reason" => "statement_timeout",
                "kind" => kind
            )
            .increment(1);
            warn!(
                user_id = %viewer.user_id,
                kind,
                statuses = ?statuses,
                elapsed_secs = elapsed.as_secs_f64(),
                "T3 UI full listing cancelled by its statement timeout"
            );
            "statement_timeout"
        }
        Err(_) => "error",
    };
    metrics::histogram!(LISTING_DURATION, "outcome" => outcome, "kind" => kind)
        .record(elapsed.as_secs_f64());
    if timed_out {
        return Err(ListingError::TimedOut);
    }
    Ok(result?)
}

/// The listing narrowed to `task_ids`: a key lookup, so it takes no permit.
pub(crate) async fn list_by_ids(
    pool: &DbPool,
    viewer: &ScopeContext,
    reach: Reach,
    statuses: &[TaskStatus],
    task_ids: &[String],
) -> Result<Vec<ListedTask>, sqlx::Error> {
    read(pool, viewer, reach, statuses, Some(task_ids), None, None).await
}

/// The one listing query at a set of statuses, optionally narrowed to a set
/// of Tasks or a browse filter, and read a `page` at a time.
///
/// Every narrowing is a `WHERE` clause on this query, so the read and follow
/// rules cannot drift between a snapshot and a live re-read of it. Unnarrowed
/// by `task_ids`, it runs only through [`list`].
async fn read(
    pool: &DbPool,
    viewer: &ScopeContext,
    reach: Reach,
    statuses: &[TaskStatus],
    task_ids: Option<&[String]>,
    browse: Option<&ListingFilter>,
    page: Option<Page<'_>>,
) -> Result<Vec<ListedTask>, sqlx::Error> {
    let before = page.as_ref().and_then(|page| page.before);
    let sql = statement(reach, statuses, task_ids.is_some(), browse, page.as_ref());
    let query = bind(
        sqlx::query_as::<_, ListedTask>(AssertSqlSafe(sql)),
        viewer,
        task_ids,
        browse,
        before,
    );
    // A narrowed read is a key lookup whatever the ids, so it takes the cached
    // generic plan. Under `auto` Postgres never switches to one for this
    // statement and plans its ~25 relations on every call, 5-400 ms against
    // ~1 ms to execute, once a second per open thread via `ThreadFeed::poll`.
    let (setting, value) = match task_ids {
        Some(_) => ("plan_cache_mode", "force_generic_plan"),
        None => ("statement_timeout", FULL_LISTING_STATEMENT_TIMEOUT),
    };
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config($1, $2, true)")
        .bind(setting)
        .bind(value)
        .execute(&mut *tx)
        .await?;
    let rows = query.fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(rows)
}

/// The SQL of [`read`]; [`bind`] fills its parameters in the order it
/// allocates them.
pub(super) fn statement(
    reach: Reach,
    statuses: &[TaskStatus],
    narrowed: bool,
    browse: Option<&ListingFilter>,
    page: Option<&Page<'_>>,
) -> String {
    // `$1`/`$2` are the access filter's admin flag and user id; `$3` is
    // the follower. The read rule is a shared fragment, not a copy, so a
    // change to "may read" reaches this listing too. The status filter
    // uses the enum's own spelling, not a literal: `tasks.status` has no
    // CHECK constraint, so a typo here would silently return nothing.
    let statuses = status_list(statuses);
    let access = moatless_core::db::access_control::access_filter(ScopedEntity::Task, "t.", 1);
    let follower = access.next_param();
    let parent_join = listed_parent_join();
    let children_join = listed_children_join();
    let question_tool = follower + 1;
    let plan_tool = follower + 2;
    // Every clause below allocates the next position in turn, and the binds
    // at the bottom of this function follow the same order.
    let mut next_param = plan_tool;
    let mut param = move || {
        next_param += 1;
        next_param
    };
    let narrowed = if narrowed {
        format!("\n                 AND t.task_id = ANY(${})", param())
    } else {
        String::new()
    };
    // Each browse filter becomes a clause only when set, not a `$n IS
    // NULL OR` pair: an unset one must disappear so the index behind it
    // stays reachable. Predicates only — the cap is appended after them
    // and holds no parameter, so a filter added here can't land behind
    // `LIMIT` and fail at runtime instead of at compile time.
    let mut predicates: Vec<String> = Vec::new();
    if let Some(browse) = browse {
        if browse.owner_user_id.is_some() {
            predicates.push(format!(
                "AND COALESCE(t.owner_user_id, t.created_by) = ${}",
                param()
            ));
        }
        if browse.tag.is_some() {
            predicates.push(format!(
                "AND EXISTS (
                          SELECT 1 FROM task_tags tt
                          JOIN tags tg ON tg.tag_id = tt.tag_id
                          WHERE tt.task_id = t.task_id AND tg.name = ${}
                      )",
                param()
            ));
        }
    }
    // Keyset rather than `OFFSET`, so a Task closed between two pages
    // shifts no row across the boundary. `-infinity` stands in for a NULL
    // `updated_at`, which `NULLS LAST` sorts in the same place.
    if page.and_then(|page| page.before).is_some() {
        let (at, id) = (param(), param());
        predicates.push(format!(
                "AND (COALESCE(t.updated_at, '-infinity') < COALESCE(${at}::timestamptz, '-infinity')
                       OR (COALESCE(t.updated_at, '-infinity') = COALESCE(${at}::timestamptz, '-infinity')
                           AND t.task_id > ${id}))"
            ));
    }
    let predicates = predicates
        .iter()
        .flat_map(|predicate| ["\n                 ", predicate.as_str()])
        .collect::<String>();
    // The cap is inside `picked`, which is also where the order it takes the
    // newest rows by has to live: the outer `ORDER BY` runs after the
    // laterals, and ordering there would cap an arbitrary set.
    let capped = page.map_or_default(|Page { cap, .. }| {
        format!(
            "\n                 ORDER BY t.updated_at DESC NULLS LAST, \
                     t.task_id ASC LIMIT {cap}"
        )
    });
    // The follow rule is the join, so widening past it is the join kind and
    // nothing else: `${follower}` moves into the `ON` clause, leaving the
    // status rule, the read rule and the narrowing exactly as they are, at
    // the same parameter positions.
    let follows = match reach {
        Reach::Followed => format!(
            "FROM task_follows tf
                   JOIN tasks t ON t.task_id = tf.task_id
                   WHERE tf.user_id = ${follower}
                     AND"
        ),
        Reach::Readable => format!(
            "FROM tasks t
                   LEFT JOIN task_follows tf
                     ON tf.task_id = t.task_id AND tf.user_id = ${follower}
                   WHERE"
        ),
    };
    // `picked` reads only columns `tasks_listing_pick_reporter_idx` covers, so
    // it never visits the heap; `listed` fetches whole rows for the picked
    // ones alone.
    // Picking with `t.*` read every followed Task's row to keep 201, and timed
    // out a viewer following thousands. A picking clause on a column the index
    // does not cover brings that back: add the column to its `INCLUDE`.
    format!(
        r#"WITH picked AS MATERIALIZED (
                   SELECT t.task_id
                   {follows} t.status IN ({statuses})
                     AND {}{narrowed}{predicates}{capped}
               ),
               listed AS MATERIALIZED (
                   SELECT t.*, tf.settled_override, tf.settled_at, tf.snoozed_until, tf.snoozed_at,
                          tf.pinned_at, tf.pin_order_key, tf.active_order_key,
                          tf.auto_settle_disabled_at
                   FROM picked
                   JOIN tasks t ON t.task_id = picked.task_id
                   LEFT JOIN task_follows tf
                     ON tf.task_id = t.task_id AND tf.user_id = ${follower}
               )
               SELECT
                   t.task_id,
                   t.workspace_id,
                   t.name,
                   t.branch,
                   t.agent_profile,
                   t.created_at,
                   t.created_at_db,
                   t.updated_at,
                   t.closed_at,
                   t.visibility,
                   COALESCE(owner.name, owner.login) AS owner_name,
                   turn.turn_number,
                   turn.status                AS turn_status,
                   turn.started_at            AS turn_started_at,
                   turn.ended_at              AS turn_ended_at,
                   turn.stop_unresponsive_at  AS turn_stop_unresponsive_at,
                   failure.turn_error,
                   failure.turn_error_reason,
                   failure.turn_error_resets_at,
                   s.desired_state            AS sandbox_desired_state_raw,
                   s.stop_reason              AS sandbox_stop_reason_raw,
                   s.started_at               AS sandbox_started_at,
                   s.desired_state_updated_at AS sandbox_updated_at,
                   COALESCE(s.running_commands, 0) AS running_commands,
                   COALESCE(commands.labels, '[]'::jsonb) AS running_command_labels,
                   COALESCE(children.child_count, 0) AS child_count,
                   person.latest_user_message_at,
                   queued.latest_queued_message_at,
                   model.assistant_model,
                   model.requested_model,
                   COALESCE(pending.question, false) AS pending_question,
                   COALESCE(pending.plan, false)     AS pending_plan,
                   t.settled_override,
                   t.settled_at,
                   t.snoozed_until,
                   t.snoozed_at,
                   receipt.last_read_at,
                   t.pinned_at,
                   t.pin_order_key,
                   t.active_order_key,
                   t.auto_settle_disabled_at,
                   COALESCE(pr.pull_requests, '[]'::jsonb) AS pull_requests,
                   COALESCE(tagged.names, '[]'::jsonb) AS tags,
                   parent.task_id AS parent_task_id,
                   parent.relation_type AS parent_relation,
                   parent.fork_at_turn,
                   COALESCE(parent.side_chat, false) AS side_chat
               FROM listed t
               LEFT JOIN sandboxes s ON s.sandbox_id = t.sandbox_id
               -- Whose Task this is, for the rows that are somebody else's. The
               -- viewer test is in the `ON` clause rather than a `CASE`, so a
               -- viewer's own Task simply misses the join and reports no name.
               -- `users.id` casts to text, not the owner to `uuid`, which would
               -- raise on a non-uuid value.
               LEFT JOIN users owner
                 ON owner.id::text = COALESCE(t.owner_user_id, t.created_by)
                AND COALESCE(t.owner_user_id, t.created_by) IS DISTINCT FROM ${follower}
               LEFT JOIN task_read_receipts receipt
                 ON receipt.task_id = t.task_id AND receipt.user_id = ${follower}
               LEFT JOIN LATERAL (
                   SELECT tt.turn_number, tt.status, tt.started_at, tt.ended_at,
                          tt.stop_unresponsive_at
                   FROM task_turns tt
                   WHERE tt.task_id = t.task_id
                   ORDER BY tt.turn_number DESC
                   LIMIT 1
               ) turn ON TRUE
               -- `queued` and `running` are the pod's non-terminal states;
               -- `running_labels` in sandbox-control names the same two.
               LEFT JOIN LATERAL (
                   SELECT jsonb_agg(c->>'label' ORDER BY (c->>'startedAtUnixMs')::bigint, c->>'id')
                          AS labels
                   FROM sandbox_unreported_commands u,
                        jsonb_array_elements(u.commands) c
                   WHERE u.task_id = t.task_id
                     AND c->>'state' IN ('queued', 'running')
               ) commands ON TRUE
               LEFT JOIN LATERAL (
                   SELECT (e.payload::jsonb)->>'error' AS turn_error,
                          (e.payload::jsonb)->>'reason' AS turn_error_reason,
                          (e.payload::jsonb)->>'resetsAt' AS turn_error_resets_at
                   FROM task_events e
                   WHERE turn.status = 'error'
                     AND e.task_id = t.task_id
                     AND e.turn_number = turn.turn_number
                     AND e.type IN ('agent', 'taskTurn')
                     AND e.sub_type = 'error'
                   ORDER BY e.id DESC
                   LIMIT 1
               ) failure ON TRUE
               LEFT JOIN LATERAL (
                   SELECT m.created_at AS latest_user_message_at
                   FROM conversation_messages m
                   JOIN task_inbox i ON i.id = m.inbox_id
                   WHERE m.task_id = t.task_id
                     AND m.author = 'user'
                     AND i.origin <> 'sandbox'
                   ORDER BY m.created_at DESC
                   LIMIT 1
               ) person ON TRUE
               LEFT JOIN LATERAL (
                   SELECT m.created_at AS latest_queued_message_at
                   FROM task_inbox m
                   WHERE m.task_id = t.task_id
                     AND m.status IN ('pending', 'delivered')
                   ORDER BY m.created_at DESC
                   LIMIT 1
               ) queued ON TRUE
               -- A user Message's requested model is its inbox item's.
               LEFT JOIN LATERAL (
                   SELECT
                       (SELECT a.model
                          FROM conversation_messages a
                         WHERE a.task_id = t.task_id
                           -- NULL = NULL is never true, so a conversation with
                           -- no turn reports no model here and falls through to
                           -- the queued-message branch below. Intended.
                           AND a.turn_number = turn.turn_number
                           AND a.author = 'agent'
                           AND a.model IS NOT NULL
                         ORDER BY a.position DESC
                         LIMIT 1) AS assistant_model,
                       COALESCE(
                           (SELECT i.delivery_options ->> 'model'
                              FROM conversation_messages u
                              JOIN task_inbox i ON i.id = u.inbox_id
                             WHERE u.task_id = t.task_id
                               AND u.turn_number = turn.turn_number
                               -- Implied by the join, but stated so the planner
                               -- can use `conversation_messages_turn_inbox_idx`;
                               -- without it, every agent Message is fetched.
                               AND u.inbox_id IS NOT NULL
                               AND i.delivery_options ->> 'model' IS NOT NULL
                             ORDER BY u.position ASC
                             LIMIT 1),
                           (SELECT i.delivery_options ->> 'model'
                              FROM conversation_messages u
                              JOIN task_inbox i ON i.id = u.inbox_id
                             WHERE u.task_id = t.task_id
                               AND u.turn_number IS NULL
                               AND u.inbox_id IS NOT NULL
                               AND i.delivery_options ->> 'model' IS NOT NULL
                             ORDER BY u.position DESC
                             LIMIT 1)
                       ) AS requested_model
               ) model ON TRUE
               -- Unanswered is read off the call's own row: an activity leaves
               -- `in_progress` when its result lands, so this needs no search
               -- for the answer.
               --
               -- Only the newest turn that did anything is scanned. An
               -- unanswered question the agent then called past stops counting
               -- as pending, which is the narrow reading: the agent has moved
               -- on. "Newest" is the highest `position` — the conversation's
               -- own ordering, allocated at write time, so unlike the `id` this
               -- used to read it is not a serial made visible at COMMIT.
               --
               -- What this no longer clears on is text: a question followed by
               -- a reply that called nothing keeps reading as pending until the
               -- agent's next call. A Message carries no position, so the two
               -- cannot be ordered against each other in one index scan, and
               -- the case is the agent answering its own question in words —
               -- which leaves a person with something to click either way.
               LEFT JOIN LATERAL (
                   SELECT
                       bool_or(raised.summary = ${question_tool}) AS question,
                       bool_or(raised.summary = ${plan_tool})     AS plan
                   FROM conversation_activities raised
                   WHERE raised.task_id = t.task_id
                     AND raised.parent_tool_use_id IS NULL
                     AND raised.kind = 'tool_call'
                     AND raised.status = 'in_progress'
                     AND raised.summary IN (${question_tool}, ${plan_tool})
                     AND raised.message_uuid = (
                           SELECT newest.message_uuid
                           FROM conversation_activities newest
                           WHERE newest.task_id = t.task_id
                             AND newest.parent_tool_use_id IS NULL
                             AND newest.kind = 'tool_call'
                           ORDER BY newest.position DESC
                           LIMIT 1
                         )
               ) pending ON TRUE
               -- The pull requests the thread row links to, read from the
               -- Task's GitHub PR bindings rather than from its branch: the
               -- agent commits on whatever branch it cut, and `tasks.branch`
               -- holds the base the Task was created on, so a branch-matched
               -- read reports nothing for most Tasks that have a PR. A Task can
               -- carry several — the unique index keys on `subject_key`, and
               -- nothing retires the binding a new one lands beside.
               --
               -- `subject_key` is `owner/repo#N`, matched by
               -- `PULL_REQUEST_KEY_PATTERN`; the guard is what makes the
               -- `::bigint` casts total, so a malformed key drops its row
               -- instead of failing the whole listing.
               --
               -- `subject_key` is the only column a row needs to be reportable,
               -- so a binding links before its status blob is fetched. Reading
               -- a key out of that blob as a *condition* would hide every
               -- freshly opened pull request: the blob is written NULL and
               -- filled in by a refresh this query cannot trigger.
               LEFT JOIN LATERAL (
                   SELECT jsonb_agg(
                              jsonb_build_object(
                                  'repository', bound.repository,
                                  'number', bound.number,
                                  'url', bound.url,
                                  'title', bound.title,
                                  'base_ref', bound.base_ref,
                                  'head_ref', bound.head_ref,
                                  'updated_at', bound.updated_at,
                                  'checks_state', bound.checks_state,
                                  'state_kind', bound.state_kind,
                                  'status_read_at', bound.status_read_at,
                                  'linked_at', bound.linked_at,
                                  'origin', bound.origin,
                                  'bind_reason', bound.bind_reason,
                                  'review_decision', bound.review_decision,
                                  'mergeable_state', bound.mergeable_state,
                                  'watching', bound.watching
                              )
                              ORDER BY bound.live DESC, bound.number DESC
                          ) AS pull_requests
                   FROM (
                       SELECT
                           split_part(b.subject_key, '#', 1) AS repository,
                           split_part(b.subject_key, '#', 2)::bigint AS number,
                           COALESCE(
                               NULLIF(b.external_status_raw ->> 'url', ''),
                               'https://github.com/' || split_part(b.subject_key, '#', 1)
                                   || '/pull/' || split_part(b.subject_key, '#', 2)::bigint
                           ) AS url,
                           NULLIF(b.external_status_raw ->> 'title', '')  AS title,
                           NULLIF(b.external_status_raw ->> 'baseRef', '') AS base_ref,
                           NULLIF(b.external_status_raw ->> 'headRef', '') AS head_ref,
                           NULLIF(b.external_status_raw ->> 'updatedAt', '') AS updated_at,
                           NULLIF(b.external_status_raw ->> 'checksState', '') AS checks_state,
                           b.external_state_kind AS state_kind,
                           COALESCE(b.external_status_updated_at, b.last_event_at)
                               AS status_read_at,
                           b.created_at AS linked_at,
                           b.origin AS origin,
                           b.metadata ->> 'bind_reason' AS bind_reason,
                           b.external_review_decision AS review_decision,
                           b.external_mergeable_state AS mergeable_state,
                           b.watching,
                           (COALESCE(b.external_state_kind, 'open')
                                NOT IN ('merged', 'closed')) AS live
                       FROM task_bindings b
                       WHERE b.task_id = t.task_id
                         AND b.active
                         AND b.adapter_kind = 'github'
                         AND b.subject_kind = 'github_pr'
                         AND b.subject_key ~ '{PULL_REQUEST_KEY_PATTERN}'
                   ) bound
               ) pr ON TRUE
               LEFT JOIN LATERAL (
                   SELECT jsonb_agg(tg.name ORDER BY tg.name) AS names
                   FROM task_tags tt
                   JOIN tags tg ON tg.tag_id = tt.tag_id
                   WHERE tt.task_id = t.task_id
               ) tagged ON TRUE
               {parent_join}
               {children_join}
               -- Determinism for this query's own output, and nothing more.
               -- This is *not* the sidebar's order: T3 sorts client-side, in
               -- `resolveSettledTimestamp` (`Sidebar.logic.ts:484-502`), from
               -- the maximum of `latestUserMessageAt` and the three
               -- `latestTurn` stamps, falling back to `updatedAt` only when all
               -- four are absent. Reading this line as the sidebar's order
               -- leads to trying to keep the server's ordering live, which
               -- nothing observes.
               ORDER BY t.updated_at DESC NULLS LAST, t.task_id ASC"#,
        access.sql(),
    )
}

/// Binds [`statement`]'s parameters, in the order it allocated them.
pub(super) fn bind<'q, O>(
    query: QueryAs<'q, Postgres, O, PgArguments>,
    viewer: &'q ScopeContext,
    task_ids: Option<&[String]>,
    browse: Option<&ListingFilter>,
    before: Option<&ListingCursor>,
) -> QueryAs<'q, Postgres, O, PgArguments> {
    let mut query = query
        .bind(viewer.is_admin)
        .bind(&viewer.user_id)
        .bind(&viewer.user_id)
        .bind(QUESTION_TOOL)
        .bind(PLAN_TOOL);
    // Bound in the order the clauses above allocated their positions.
    if let Some(task_ids) = task_ids {
        query = query.bind(task_ids.to_vec());
    }
    if let Some(browse) = browse {
        if let Some(owner_user_id) = browse.owner_user_id.clone() {
            query = query.bind(owner_user_id);
        }
        if let Some(tag) = browse.tag.clone() {
            query = query.bind(tag);
        }
    }
    if let Some(before) = before {
        query = query.bind(before.updated_at).bind(before.task_id.clone());
    }
    query
}

/// The listing's `LEFT JOIN LATERAL … parent`: each row's first parent edge,
/// picked as `TaskRelationSource::parent` picks it, named only when the viewer
/// may read the parent (the listing's `$1`/`$2`). Following the parent is not
/// required: the client lists a child on its own and links up to the parent.
fn listed_parent_join() -> String {
    let access = moatless_core::db::access_control::access_filter(
        moatless_types::access_control::ScopedEntity::Task,
        "p.",
        1,
    );
    let access = access.sql();
    let parent = TaskRelationType::PARENT_SQL_LIST;
    format!(
        "LEFT JOIN LATERAL (
            SELECT p.task_id, edge.relation_type,
                   CASE WHEN jsonb_typeof(edge.context -> 'at_turn') = 'number'
                         AND (edge.context ->> 'at_turn')::numeric BETWEEN 1 AND 2147483647
                        THEN (edge.context ->> 'at_turn')::numeric::int
                   END AS fork_at_turn,
                   (edge.relation_type = 'forked_from' AND edge.context -> 'side_chat' = 'true'::jsonb) AS side_chat
            FROM (
                SELECT r.to_task_id, r.relation_type, r.context
                FROM task_relations r
                WHERE r.from_task_id = t.task_id
                  AND r.relation_type IN {parent}
                ORDER BY r.created_at ASC
                LIMIT 1
            ) edge
            JOIN tasks p ON p.task_id = edge.to_task_id
            WHERE {access}
        ) parent ON TRUE"
    )
}

/// The listing's `children` join: how many readable Tasks this row spawned or
/// was forked into (`child_count`, the set `subtasks.list` returns).
fn listed_children_join() -> String {
    let access = moatless_core::db::access_control::access_filter(
        moatless_types::access_control::ScopedEntity::Task,
        "c.",
        1,
    );
    let access = access.sql();
    let parent = TaskRelationType::PARENT_SQL_LIST;
    format!(
        "LEFT JOIN LATERAL (
            SELECT count(DISTINCT c.task_id) AS child_count
            FROM task_relations r
            JOIN tasks c ON c.task_id = r.from_task_id
            WHERE r.to_task_id = t.task_id
              AND r.relation_type IN {parent}
              AND (r.relation_type <> 'forked_from'
                   OR r.context -> 'side_chat' IS DISTINCT FROM 'true'::jsonb)
              AND {access}
        ) children ON TRUE"
    )
}
