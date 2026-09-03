-- Stamp `sent_at` on Messages the agent has already taken.
--
-- Two columns answer "has the agent got this". Every dispatch predicate reads
-- `agent_session_id` (`get_pending_user`, `members_awaiting_dispatch`,
-- `clear_settled_pending_work`); the conversation's queued badge reads
-- `sent_at` (`message/transform.rs`). The two writers that claim a Message set
-- both in one statement, so they agree — but the fork copy inserted through a
-- column list that named `agent_session_id` and not `sent_at`, leaving each
-- copied Message claimed by one and unsent by the other. It reads as queued
-- forever while nothing will ever dispatch it again.
--
-- `created_at`, not the Message's own `timestamp`: that column is text, and a
-- row whose text does not parse would abort this migration. The value only has
-- to be a time the Message already existed at, because `sent_at` has no reader
-- but the badge.

UPDATE task_messages
   SET sent_at = created_at
 WHERE message_type IN ('user', 'error')
   AND agent_session_id IS NOT NULL
   AND sent_at IS NULL;
