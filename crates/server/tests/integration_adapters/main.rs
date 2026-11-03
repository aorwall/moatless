//! Integration tests over the Adapter tier — ingress, delivery, installation.
//! One of three `integration_*` test targets; shared fixtures live in
//! `tests/common`.

// Every module below carries its own `#![cfg(feature = "db-tests")]`,
// so `common` is unreachable without it.
#[cfg(feature = "db-tests")]
#[path = "../common/mod.rs"]
mod common;

mod adapter_connection_delete_test;
mod adapter_delivery_github_linear_test;
mod adapter_inbound_delivery_test;
mod adapter_installation_test;
mod default_branch_watch_test;
mod event_ingress_test;
mod github_push_claim_test;
mod github_webhook_redelivery_test;
mod inbound_message_buffer_test;
mod linear_admission_test;
mod linear_delivery_identity_test;
mod linear_factory_test;
mod linear_ingress_test;
mod linear_webhook_metrics_test;
mod linear_workflow_test;
mod loops_sync_test;
mod slack_ingress_order_test;
mod task_room_open_test;
