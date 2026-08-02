# Conversation storage migration

Moving a Task's conversation from one jsonb document per Message to two tables —
`task_messages` for what somebody said, `task_activities` for what happened on
the way. Spec: `.plans/completed/event-sourced-conversation.md`.

Migration `0118_task_activities.sql` creates everything and drops nothing. It
runs at boot and is safe on its own. **Everything below is operator-run**, in
this order, from a psql session against the primary.

## What is reversible, and where that stops

| Step | Reversible? |
|---|---|
| 1 · Fold text into `message_text` | Yes — `message_content` still holds every document |
| 2 · Backfill `task_message_usage` | Yes — derived from rows still present |
| 3 · Delete the rows that are not conversation | Yes — restore from a snapshot |
| 4 · Delete `agent.message` skeletons | Yes — restore from a snapshot |
| **go/no-go** | |
| 5 · `DROP COLUMN message_content` | **No** |
| 6 · `DROP TABLE task_message_content_blocks` | **No** |
| 7 · `VACUUM FULL` | n/a |

Steps 1–4 are batched, idempotent and resumable: each batch is its own
transaction and the predicate *is* the progress, so an interrupted run is
restarted by running the same statement again. Steps 5 and 6 are single DDL
statements that delete the source this change was migrated from. After them,
rolling back means writing tool blocks back from `task_activities` — a
migration, not a revert.

## Before you start

The deployed backend already writes both shapes and reads only the new one, so
a Task whose text has not been folded **renders blank** until step 1 reaches
it. Run step 1 promptly after the deploy; it is online and takes no exclusive
lock.

Check where you are:

```sql
SELECT
    count(*) FILTER (WHERE message_type IN ('user', 'assistant'))            AS conversation_rows,
    count(*) FILTER (WHERE message_type IN ('user', 'assistant')
                       AND message_text IS NULL)                             AS unfolded,
    count(*) FILTER (WHERE message_type NOT IN ('user', 'assistant'))        AS not_conversation
FROM task_messages;
```

## 1 · Fold text into `message_text`

The text blocks of `message_content` in `sort_order`, joined with a blank line —
which is what `crates/t3code/src/projection.rs` computed on every read and what
`message/transform.rs` produced by rendering one markdown block each.

Only `user` and `assistant`: both readers already dropped the other three
(Decision 13). Run until it reports `UPDATE 0`.

```sql
WITH batch AS (
    SELECT id
    FROM task_messages
    WHERE message_type IN ('user', 'assistant')
      AND message_text IS NULL
      AND message_content IS NOT NULL
    ORDER BY id
    LIMIT 5000
),
folded AS (
    SELECT b.id,
           (SELECT string_agg(block ->> 'text', E'\n\n' ORDER BY ordinality)
            FROM task_messages m,
                 LATERAL jsonb_array_elements(m.message_content)
                     WITH ORDINALITY AS t(block, ordinality)
            WHERE m.id = b.id
              AND jsonb_typeof(m.message_content) = 'array'
              AND block ->> 'type' = 'text'
              AND coalesce(block ->> 'text', '') <> '') AS text
    FROM batch b
)
UPDATE task_messages m
SET message_text = coalesce(
        folded.text,
        -- A Message written as a bare string rather than a block list.
        CASE WHEN jsonb_typeof(m.message_content) = 'string'
             THEN m.message_content #>> '{}' END,
        ''),
    message_text_parts = NULL
FROM folded
WHERE m.id = folded.id;
```

`''` rather than NULL for a Message with no text: NULL is what the predicate
reads as "not folded yet", so a row that folds to nothing would be re-selected
for ever. An empty `message_text` renders no bubble, which is correct — an
assistant turn that only called tools said nothing.

`message_text_parts` is left NULL on purpose. It is the splice buffer for blocks
still arriving; a Message old enough to be folded takes no more.

**Verify.** `unfolded` from the query above must be 0.

## 2 · Backfill `task_message_usage`

`get_subagent_aggregates` puts a token count on an `Agent` tool call, and it
summed columns on rows step 3 deletes. Lift the sums out first. Idempotent by
primary key; run until `INSERT 0 0`.

```sql
INSERT INTO task_message_usage (message_uuid, task_id, parent_uuid, input_tokens, output_tokens)
SELECT m.uuid, m.task_id, m.parent_uuid,
       coalesce(m.assistant_usage_input_tokens, 0),
       coalesce(m.assistant_usage_output_tokens, 0)
FROM task_messages m
WHERE m.message_type = 'assistant'
  AND (m.assistant_usage_input_tokens IS NOT NULL
       OR m.assistant_usage_output_tokens IS NOT NULL)
  AND NOT EXISTS (SELECT 1 FROM task_message_usage u WHERE u.message_uuid = m.uuid)
LIMIT 20000;
```

**Verify.** The count must be non-zero and stable:

```sql
SELECT count(*) FROM task_message_usage;
```

## 3 · Delete the rows that are not conversation

A Message row exists when somebody said something. `system` rows carried tool
results, `result` and `error` rows carried the run's own closing line, and both
readers already dropped all three. The 153 rows holding the literal text `{}`
go with them.

`task_message_content_blocks` and `task_message_contexts` cascade on the foreign
key, so this shrinks both. Run until `DELETE 0`.

```sql
DELETE FROM task_messages
WHERE id IN (
    SELECT id FROM task_messages
    WHERE message_type NOT IN ('user', 'assistant')
       OR (message_type IN ('user', 'assistant') AND message_text = '{}')
    LIMIT 5000
);
```

> A conversation whose *whole* history is these rows empties. That is the
> intended outcome and it is what Decision 13 accepts: those rows are not a
> Message anybody sent.

## 4 · Delete the message skeletons from the event log

163,141 of 1,763,348 `task_events` rows are `agent.message` — a skeleton of an
assistant message the Events tab timeline draws and nothing else reads. The
conversation is the conversation's own record now. Every other event type stays,
including `taskTurn`, `agent`, `server` and `workspace_synced`. Run until
`DELETE 0`.

```sql
DELETE FROM task_events
WHERE id IN (
    SELECT id FROM task_events WHERE "type" = 'agent.message' LIMIT 10000
);
```

## go/no-go

Do not continue until all four hold:

1. `unfolded` is 0 and has been 0 across a full deploy cycle.
2. A conversation opened in the UI shows its messages, its tool calls, its
   thinking and its Sandbox lifecycle.
3. Task statistics for a Task written *after* the deploy report tool counts
   (`toolCallsAvailable` is true).
4. You have a database snapshot you are willing to restore from.

Steps 5 and 6 delete the source. There is no rollback after them.

## 5 · Drop `message_content`

```sql
ALTER TABLE task_messages DROP COLUMN message_content;
```

Catalogue-only in PostgreSQL — instant, and the space comes back at step 7.

## 6 · Drop the block table

```sql
DROP TABLE task_message_content_blocks;
```

Both of its readers are gone: the statistics counters read
`task_activities.is_error`, and the sibling fold at the old
`message/dao.rs:1155` does not exist — a call and its outcome are one row.

## 7 · Reclaim the space

`VACUUM FULL` takes an `ACCESS EXCLUSIVE` lock for the whole rewrite. **Run it
in a maintenance window.** While it holds the lock, everything that does not
read conversation content keeps working — the shell listing, the event log,
message sending, every Sandbox write path. What stalls is conversation assembly.

```sql
VACUUM FULL ANALYZE task_messages;
VACUUM FULL ANALYZE task_events;
```

`task_message_content_blocks` needs neither: dropping a table returns its files
directly.

Expected after: `task_messages` from 4,143 MB to roughly 200 MB, and 1,861 MB
returned from the block table.

```sql
SELECT relname, pg_size_pretty(pg_total_relation_size(oid))
FROM pg_class
WHERE relname IN ('task_messages', 'task_activities', 'task_events', 'task_message_usage')
ORDER BY pg_total_relation_size(oid) DESC;
```

## What an old conversation looks like afterwards

4,962 conversations written before the deploy show their messages and **none of
their tool calls, thinking or Sandbox lifecycle** (Decision 9: no activity
backfill — an activity backfill would have to invent a kind mapping, a status
inference and ids, where the text fold invents nothing). Their statistics say
the counts are unknown rather than reporting zero as a fact.

This is expected. It is not a failed migration.
