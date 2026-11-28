//! Shared Workspace, repository, and plugin values.

#![cfg_attr(
    test,
    expect(
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "tests fail by panicking"
    )
)]

pub(crate) use moatless_types::string_enum;

pub mod plugin;
pub mod repository;
#[cfg(feature = "schemas")]
pub mod schemas;
pub mod workspace;
