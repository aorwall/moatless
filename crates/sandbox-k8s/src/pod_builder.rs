//! Code-generated Pod spec.
//!
//! The full spec is generated in Rust rather than from a YAML template.

use super::naming::generate_pod_name;
use k8s_openapi::api::core::v1::{
    Affinity, Capabilities, Container, ContainerPort, EmptyDirVolumeSource, EnvVar, HTTPGetAction,
    HostPathVolumeSource, NodeAffinity, NodeSelectorRequirement, NodeSelectorTerm,
    PersistentVolumeClaimVolumeSource, Pod, PodSecurityContext, PodSpec, PreferredSchedulingTerm,
    Probe, ResourceRequirements, SeccompProfile, SecurityContext, TCPSocketAction, Toleration,
    Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use moatless_core::config::Config;
use moatless_core::sandbox_jwt::{self as jwt, MintedSandboxTokens};
use moatless_sandbox_runtime::error::SandboxError;
use moatless_sandbox_runtime::provisioning::ProvisionPlan;
use moatless_sandbox_runtime::runtime::{OPERATION_GENERATION_LABEL, RuntimeOperationContext};
use moatless_types::sandbox::SANDBOX_PREVIEW_PROXY_PORT;
use moatless_workspace_types::workspace::{
    Cpu, Memory, ResourceConfig, ResourceQuantities, StorageMedium,
};
use moatless_workspace_types::workspace::{RepoCheckout, WORKSPACE_LIMIT_ENV, WORKSPACE_REPOS_ENV};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tracing::{debug, info, warn};

mod containers;
mod env;
mod placement;
mod pod;
mod security;
mod tokens;

#[cfg(test)]
mod tests;

pub use placement::resolve_sandbox_image;
pub(crate) use placement::{pin_pod_to_node, retained_node_affinity};
pub use pod::{
    POD_SPEC_HASH_ANNOTATION, build_pod, pod_spec_fingerprint, stamp_operation_generation,
};
pub use tokens::{SandboxAuthTokens, mint_sandbox_tokens};
