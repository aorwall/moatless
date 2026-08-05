//! Every backend integration test compiles into this one binary, because each
//! target links the full backend closure and linking dominates the build.
//! nextest runs each test in its own process, so isolation is unchanged.

// Every module below except `supervisor_test` carries its own
// `#![cfg(feature = "db-tests")]`, so `common` is unreachable without it.
#[cfg(feature = "db-tests")]
mod common;

mod adapter_connection_delete_test;
mod adapter_delivery_github_linear_test;
mod adapter_from_config_test;
mod adapter_inbound_delivery_test;
mod agent_handlers_hooks_test;
mod agent_handlers_per_block_test;
mod closed_terminal_test;
mod device_pairing_test;
mod event_ingress_test;
mod feedback_dao_test;
mod github_app_registration_test;
mod github_webhook_redelivery_test;
mod linear_admission_test;
mod linear_delivery_identity_test;
mod linear_ingress_test;
mod linear_webhook_metrics_test;
mod linear_workflow_test;
mod loops_sync_test;
mod message_dao_test;
mod message_handler_test;
mod plugin_skills_dao_test;
mod repository_dao_test;
mod repository_handler_test;
mod slack_ingress_order_test;
mod supervisor_test;
mod tag_dao_test;
mod tag_handler_test;
mod task_bindings_port_test;
mod task_dao_test;
mod task_event_dao_test;
mod task_handler_test;
mod task_listing_position_test;
mod task_turn_dao_test;
mod telegram_ingress_test;
mod ui_rpc_command_test;
mod ui_rpc_preview_tabs_redis_test;
mod ui_rpc_shell_test;
mod ui_rpc_thread_test;
mod user_uuid_test;
mod workspace_dao_test;
mod workspace_shadow_test;
