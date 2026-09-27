# Conversation read model

- The Projector is the only writer of `conversation_*`. A new source of
  conversation rows writes its own table (`task_inbox`, `trajectory_events`,
  `task_events`) and calls the Projector in that transaction. The
  `conversation-writer` arch rule fails SQL that writes `conversation_*` from
  any other module.
- Derive every row id through `ids`, never `Uuid::new_v4()`: `rebuild` and a
  fork's inheritance must reproduce the same ids.
- A frozen row has no source, so `rebuild` keeps it as it is and nothing
  restores one a statement deletes or rewrites. Scope such a statement the way
  `dao`'s rewind and settle statements do.
- After changing what the Projector derives, `rebuild` a Task in a DB test and
  compare it with the rows the live path wrote (`replay_db_tests.rs`).
