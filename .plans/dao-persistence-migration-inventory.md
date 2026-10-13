# DAO persistence migration inventory

`tooling/arch-rules/src/persistence_export.rs` generates this file and holds the concern table the `persistence-export` rule enforces. Regenerate it with:

```bash
cargo run -p moatless-arch-rules --bin arch-report -- --persistence-inventory
```

A consumer count is the number of `use` declarations naming the concern's modules or its exports, and `test` counts the ones inside a test target or a test-gated item.

| Concern | Owner | Slice | Exports | Peer consumers |
|---|---|---|---|---|
| [adapter connections](#adapter-connections) | moatless-adapter-model | 02 | 0 | 0 |
| [telegram pairing](#telegram-pairing) | moatless-adapter-telegram | 15 | 0 | 0 |
| [file metadata](#file-metadata) | moatless-core | 01 | 0 | 0 |
| [conversation feed](#conversation-feed) | moatless-core | 01 | 8 | 7 |
| [events feed](#events-feed) | moatless-core | 01 | 7 | 6 |
| [listing feed](#listing-feed) | moatless-core | 01 | 5 | 8 |
| [preview feed](#preview-feed) | moatless-core | 01 | 4 | 4 |
| [terminal feed](#terminal-feed) | moatless-core | 01 | 4 | 0 |
| [users](#users) | moatless-identity | 03 | 0 | 0 |
| [teams](#teams) | moatless-identity | 03 | 0 | 0 |
| [account creation](#account-creation) | moatless-identity | 04 | 0 | 0 |
| [api keys](#api-keys) | moatless-identity | 04 | 0 | 0 |
| [passwords](#passwords) | moatless-identity | 04 | 0 | 0 |
| [oidc](#oidc) | moatless-identity | 04 | 0 | 0 |
| [device pairing](#device-pairing) | moatless-identity | 04 | 0 | 0 |
| [sessions](#sessions) | moatless-identity | 04 | 0 | 0 |
| [provider tokens](#provider-tokens) | moatless-identity | 05 | 0 | 0 |
| [secrets](#secrets) | moatless-identity | 06 | 0 | 0 |
| [secret audit](#secret-audit) | moatless-identity | 06 | 0 | 0 |
| [adapter apps](#adapter-apps) | moatless-identity | 06 | 0 | 0 |
| [forgejo instances](#forgejo-instances) | moatless-identity | 06 | 0 | 0 |
| [loop aggregate](#loop-aggregate) | moatless-loops | 13 | 0 | 0 |
| [task bindings](#task-bindings) | moatless-loops | 14 | 0 | 0 |
| [inbound message buffer](#inbound-message-buffer) | moatless-loops | 14 | 0 | 0 |
| [adapter delivery](#adapter-delivery) | moatless-loops | 15 | 0 | 0 |
| [firing receipts](#firing-receipts) | moatless-loops | 15 | 0 | 0 |
| [sandbox lifecycle](#sandbox-lifecycle) | moatless-sandbox-control | 16 | 0 | 0 |
| [t3 conversation projection](#t3-conversation-projection) | moatless-t3code | 17 | 0 | 0 |
| [t3 activity projection](#t3-activity-projection) | moatless-t3code | 17 | 0 | 0 |
| [t3 task projection](#t3-task-projection) | moatless-t3code | 17 | 0 | 0 |
| [task graph](#task-graph) | moatless-tasks | 09 | 0 | 0 |
| [task relations](#task-relations) | moatless-tasks | 09 | 0 | 0 |
| [task forks](#task-forks) | moatless-tasks | 09 | 0 | 0 |
| [artifacts](#artifacts) | moatless-tasks | 09 | 0 | 0 |
| [messages](#messages) | moatless-tasks | 10 | 0 | 0 |
| [task activities](#task-activities) | moatless-tasks | 10 | 0 | 0 |
| [outbound events](#outbound-events) | moatless-tasks | 10 | 0 | 0 |
| [task turns](#task-turns) | moatless-tasks | 11 | 0 | 0 |
| [task events](#task-events) | moatless-tasks | 11 | 0 | 0 |
| [inbox](#inbox) | moatless-tasks | 11 | 0 | 0 |
| [trajectory](#trajectory) | moatless-tasks | 11 | 0 | 0 |
| [tags](#tags) | moatless-tasks | 12 | 0 | 0 |
| [feedback](#feedback) | moatless-tasks | 12 | 0 | 0 |
| [repositories](#repositories) | moatless-workspaces | 07 | 0 | 0 |
| [workspaces](#workspaces) | moatless-workspaces | 07 | 0 | 0 |
| [workspace credentials](#workspace-credentials) | moatless-workspaces | 08 | 0 | 0 |
| [plugins](#plugins) | moatless-workspaces | 08 | 0 | 0 |

## adapter connections

- **Owner:** moatless-adapter-model
- **Closing slice:** 02
- **Persistence roots:** `crates/adapter-model/src/connections/dao.rs`, `crates/adapter-model/src/connections/dao/postgres.rs`, `crates/adapter-model/src/connections/dao/row.rs`
- **Owning tables:** `adapter_connections`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## telegram pairing

- **Owner:** moatless-adapter-telegram
- **Closing slice:** 15
- **Persistence roots:** `crates/adapter-telegram/src/pairing/dao.rs`
- **Owning tables:** `integration_pairing_requests`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## file metadata

- **Owner:** moatless-core
- **Closing slice:** 01
- **Persistence roots:** `crates/core/src/file_storage/dao.rs`
- **Owning tables:** `file_metadata`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## conversation feed

- **Owner:** moatless-core
- **Closing slice:** 01
- **Persistence roots:** `crates/core/src/change_feed/conversation/dao.rs`
- **Owning tables:** `task_conversation_changes`, `task_conversation_position`
- **Exports:** `change_feed::conversation::allocate_position`, `change_feed::conversation::changes_from`, `change_feed::conversation::current_head`, `change_feed::conversation::prune_changes`, `change_feed::conversation::record_activity_change`, `change_feed::conversation::record_message_change`, `change_feed::conversation::record_revert_change`, `change_feed::conversation::record_session_change`
- **Constructors:** none
- **Peer consumers:** moatless-tasks (5)
- **Test-only consumers:** moatless-t3code (2)

## events feed

- **Owner:** moatless-core
- **Closing slice:** 01
- **Persistence roots:** `crates/core/src/change_feed/events/dao.rs`
- **Owning tables:** `task_event_changes`, `task_event_dispatch_queue`, `task_event_position`
- **Exports:** `change_feed::events::changes_after`, `change_feed::events::claim_dispatch`, `change_feed::events::current_position`, `change_feed::events::dispatch_depth`, `change_feed::events::prune_changes`, `change_feed::events::record_event`, `change_feed::events::release_dispatch`
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** moatless-backend (6)

## listing feed

- **Owner:** moatless-core
- **Closing slice:** 01
- **Persistence roots:** `crates/core/src/change_feed/listing/dao.rs`
- **Owning tables:** `task_listing_changes`, `task_listing_position`
- **Exports:** `change_feed::listing::changes_after`, `change_feed::listing::current_head`, `change_feed::listing::current_position`, `change_feed::listing::prune_changes`, `change_feed::listing::record_change`
- **Constructors:** none
- **Peer consumers:** moatless-tasks (1)
- **Test-only consumers:** moatless-backend (7)

## preview feed

- **Owner:** moatless-core
- **Closing slice:** 01
- **Persistence roots:** `crates/core/src/change_feed/preview/dao.rs`
- **Owning tables:** `preview_tab_changes`, `preview_tab_position`
- **Exports:** `change_feed::preview::current_position`, `change_feed::preview::opens_after`, `change_feed::preview::prune_opens`, `change_feed::preview::record_open`
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** moatless-backend (4)

## terminal feed

- **Owner:** moatless-core
- **Closing slice:** 01
- **Persistence roots:** `crates/core/src/change_feed/terminal/dao.rs`
- **Owning tables:** `terminal_session_changes`, `terminal_session_position`
- **Exports:** `change_feed::terminal::current_position`, `change_feed::terminal::prune_starts`, `change_feed::terminal::record_start`, `change_feed::terminal::starts_after`
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## users

- **Owner:** moatless-identity
- **Closing slice:** 03
- **Persistence roots:** `crates/identity/src/auth/users_dao.rs`
- **Owning tables:** `user_provider_ids`, `users`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## teams

- **Owner:** moatless-identity
- **Closing slice:** 03
- **Persistence roots:** `crates/identity/src/team/dao.rs`
- **Owning tables:** `team_members`, `teams`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## account creation

- **Owner:** moatless-identity
- **Closing slice:** 04
- **Persistence roots:** `crates/identity/src/auth/account_dao.rs`
- **Owning tables:** none
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## api keys

- **Owner:** moatless-identity
- **Closing slice:** 04
- **Persistence roots:** `crates/identity/src/auth/api_keys_dao.rs`
- **Owning tables:** `api_keys`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## passwords

- **Owner:** moatless-identity
- **Closing slice:** 04
- **Persistence roots:** `crates/identity/src/auth/password/dao.rs`, `crates/identity/src/auth/password/throttle_dao.rs`
- **Owning tables:** `auth_rate_limits`, `user_credentials`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## oidc

- **Owner:** moatless-identity
- **Closing slice:** 04
- **Persistence roots:** `crates/identity/src/auth/oidc_dao.rs`
- **Owning tables:** `auth_transactions`, `oidc_identities`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## device pairing

- **Owner:** moatless-identity
- **Closing slice:** 04
- **Persistence roots:** `crates/identity/src/auth/pairing_dao.rs`
- **Owning tables:** `device_pairing_codes`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## sessions

- **Owner:** moatless-identity
- **Closing slice:** 04
- **Persistence roots:** `crates/identity/src/auth/sessions/dao.rs`
- **Owning tables:** `auth_audit_events`, `auth_sessions`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## provider tokens

- **Owner:** moatless-identity
- **Closing slice:** 05
- **Persistence roots:** `crates/identity/src/auth/provider_token_auth_dao.rs`, `crates/identity/src/auth/provider_token_dao.rs`
- **Owning tables:** `provider_tokens`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## secrets

- **Owner:** moatless-identity
- **Closing slice:** 06
- **Persistence roots:** `crates/identity/src/secret/dao.rs`
- **Owning tables:** `secrets`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## secret audit

- **Owner:** moatless-identity
- **Closing slice:** 06
- **Persistence roots:** `crates/identity/src/secret/audit_dao.rs`
- **Owning tables:** `secret_audit_events`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## adapter apps

- **Owner:** moatless-identity
- **Closing slice:** 06
- **Persistence roots:** `crates/identity/src/adapter_apps/app_dao.rs`, `crates/identity/src/adapter_apps/dao.rs`
- **Owning tables:** `adapter_app_audit_events`, `adapter_app_migrations`, `adapter_app_secrets`, `adapter_apps`, `github_app_audit_events`, `github_app_migrations`, `github_apps`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## forgejo instances

- **Owner:** moatless-identity
- **Closing slice:** 06
- **Persistence roots:** `crates/identity/src/forgejo/dao.rs`
- **Owning tables:** `forgejo_instances`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## loop aggregate

- **Owner:** moatless-loops
- **Closing slice:** 13
- **Persistence roots:** `crates/loops/src/loops/dao.rs`
- **Owning tables:** `loop_configs`, `loops`, `schedules`, `subscriptions`
- **Exports:** none
- **Constructors:** `LoopDao::new`
- **Peer consumers:** none
- **Test-only consumers:** none

## task bindings

- **Owner:** moatless-loops
- **Closing slice:** 14
- **Persistence roots:** `crates/loops/src/bindings/dao.rs`
- **Owning tables:** `task_bindings`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## inbound message buffer

- **Owner:** moatless-loops
- **Closing slice:** 14
- **Persistence roots:** `crates/loops/src/bindings/buffer/dao.rs`
- **Owning tables:** `inbound_message_buffer`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## adapter delivery

- **Owner:** moatless-loops
- **Closing slice:** 15
- **Persistence roots:** `crates/loops/src/adapters/delivery/dao.rs`
- **Owning tables:** `adapter_delivery`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## firing receipts

- **Owner:** moatless-loops
- **Closing slice:** 15
- **Persistence roots:** `crates/loops/src/loops/firing/receipts_dao.rs`
- **Owning tables:** `adapter_event_receipts`, `adapter_task_creation_claims`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## sandbox lifecycle

- **Owner:** moatless-sandbox-control
- **Closing slice:** 16
- **Persistence roots:** `crates/sandbox-control/src/lifecycle/dao.rs`, `crates/sandbox-control/src/lifecycle/db/lock.rs`, `crates/sandbox-control/src/lifecycle/db/mod.rs`
- **Owning tables:** `sandbox_unreported_commands`, `sandboxes`
- **Exports:** none
- **Constructors:** `PgLifecycleDao::new`, `PgLifecycleLock::new`
- **Peer consumers:** none
- **Test-only consumers:** none

## t3 conversation projection

- **Owner:** moatless-t3code
- **Closing slice:** 17
- **Persistence roots:** `crates/t3code/src/dao.rs`, `crates/t3code/src/transcript_dao.rs`
- **Owning tables:** none
- **Exports:** none
- **Constructors:** `ConversationBrowse::new`, `PgConversationDao::new`
- **Peer consumers:** none
- **Test-only consumers:** none

## t3 activity projection

- **Owner:** moatless-t3code
- **Closing slice:** 17
- **Persistence roots:** `crates/t3code/src/activity_dao.rs`
- **Owning tables:** none
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## t3 task projection

- **Owner:** moatless-t3code
- **Closing slice:** 17
- **Persistence roots:** `crates/t3code/src/task_dao.rs`
- **Owning tables:** none
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## task graph

- **Owner:** moatless-tasks
- **Closing slice:** 09
- **Persistence roots:** `crates/tasks/src/task/dao.rs`, `crates/tasks/src/task/pin_dao.rs`, `crates/tasks/src/task/switch_dao.rs`
- **Owning tables:** `task_follows`, `task_read_receipts`, `tasks`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## task relations

- **Owner:** moatless-tasks
- **Closing slice:** 09
- **Persistence roots:** `crates/tasks/src/task/relations_dao.rs`
- **Owning tables:** `task_relations`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## task forks

- **Owner:** moatless-tasks
- **Closing slice:** 09
- **Persistence roots:** `crates/tasks/src/task/fork_dao.rs`
- **Owning tables:** `task_forks`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## artifacts

- **Owner:** moatless-tasks
- **Closing slice:** 09
- **Persistence roots:** `crates/tasks/src/artifact/dao.rs`
- **Owning tables:** `task_artifacts`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## messages

- **Owner:** moatless-tasks
- **Closing slice:** 10
- **Persistence roots:** `crates/tasks/src/conversation/dao.rs`, `crates/tasks/src/conversation/read_dao.rs`, `crates/tasks/src/conversation/replay_dao.rs`, `crates/tasks/src/message/dao.rs`, `crates/tasks/src/message/external_events_dao.rs`, `crates/tasks/src/message/recovery_dao.rs`, `crates/tasks/src/task/rewind_dao.rs`
- **Owning tables:** `conversation_frozen`, `conversation_messages`, `conversation_usage`, `task_message_content_blocks`, `task_message_contexts`, `task_message_usage`, `task_messages`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## task activities

- **Owner:** moatless-tasks
- **Closing slice:** 10
- **Persistence roots:** `crates/tasks/src/activities/dao.rs`
- **Owning tables:** `conversation_activities`, `task_activities`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## outbound events

- **Owner:** moatless-tasks
- **Closing slice:** 10
- **Persistence roots:** `crates/tasks/src/task/outbound_events_dao.rs`
- **Owning tables:** `task_outbound_events`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## task turns

- **Owner:** moatless-tasks
- **Closing slice:** 11
- **Persistence roots:** `crates/tasks/src/task_turn/dao.rs`, `crates/tasks/src/task_turn/usage_dao.rs`
- **Owning tables:** `task_rewound_turns`, `task_turns`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## task events

- **Owner:** moatless-tasks
- **Closing slice:** 11
- **Persistence roots:** `crates/tasks/src/task_event/dao.rs`
- **Owning tables:** `task_events`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## inbox

- **Owner:** moatless-tasks
- **Closing slice:** 11
- **Persistence roots:** `crates/tasks/src/inbox/dao.rs`
- **Owning tables:** `task_inbox`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## trajectory

- **Owner:** moatless-tasks
- **Closing slice:** 11
- **Persistence roots:** `crates/tasks/src/trajectory/dao.rs`
- **Owning tables:** `trajectory_events`, `trajectory_sessions`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## tags

- **Owner:** moatless-tasks
- **Closing slice:** 12
- **Persistence roots:** `crates/tasks/src/tag/dao.rs`
- **Owning tables:** `tags`, `task_tags`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## feedback

- **Owner:** moatless-tasks
- **Closing slice:** 12
- **Persistence roots:** `crates/tasks/src/feedback/dao.rs`
- **Owning tables:** `feedback`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## repositories

- **Owner:** moatless-workspaces
- **Closing slice:** 07
- **Persistence roots:** `crates/workspaces/src/repository/dao.rs`, `crates/workspaces/src/repository/secret_migration_dao.rs`
- **Owning tables:** `repositories`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## workspaces

- **Owner:** moatless-workspaces
- **Closing slice:** 07
- **Persistence roots:** `crates/workspaces/src/workspace/dao.rs`
- **Owning tables:** `workspace_repos`, `workspaces`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## workspace credentials

- **Owner:** moatless-workspaces
- **Closing slice:** 08
- **Persistence roots:** `crates/workspaces/src/workspace/credentials/dao.rs`
- **Owning tables:** `workspace_credentials`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## plugins

- **Owner:** moatless-workspaces
- **Closing slice:** 08
- **Persistence roots:** `crates/workspaces/src/plugin/dao.rs`
- **Owning tables:** `plugin_activations`, `plugin_skills`, `plugins`
- **Exports:** none
- **Constructors:** none
- **Peer consumers:** none
- **Test-only consumers:** none

## Stores outside a persistence root

The rule reports each of these on its name alone. They sit beside the concern their file names, so the slice that closes that concern owns them too.

| Crate | Export | File |
|---|---|---|
