//! Shared API request/response types for backend and CLI.
//!
//! This crate provides the canonical set of API DTOs (Data Transfer Objects)
//! used across both the Rust backend (with `utoipa` OpenAPI support) and the
//! CLI client (serde-only).
//!
//! # Feature flags
//!
//! - `openapi` — enables `utoipa::ToSchema` / `utoipa::IntoParams` derives
//! - `database` — enables `sqlx` `Type`/`Encode`/`Decode` impls for enums

pub mod adapters;
pub mod agent;
pub mod auth;
pub mod bindings;
pub mod codex;
pub mod command;
pub mod enums;
pub mod feedback;
pub mod figma;
pub mod file_storage;
pub mod message;
pub mod plugin_repo;
pub mod provider_settings;
pub mod repository;
pub mod sandbox;
pub mod shared;
pub mod skill;
pub mod tag;
pub mod task;
pub mod task_event;
pub mod task_template;
pub mod team;
pub mod triggers;
pub mod user;
