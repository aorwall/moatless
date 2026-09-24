//! Unit tests for the pod spec, one module per part of the spec it builds.

use super::containers::build_sandbox_container;
use super::env::{chrono_now_iso, env_var, redact_url_userinfo};
use super::*;
use moatless_sandbox_runtime::provisioning::RepoConfig;

mod docker;
mod env;
mod fargate;
mod fingerprint;
mod fixtures;
mod placement;
mod probes;
mod profiles;
mod resources;
mod security;
mod storage;
