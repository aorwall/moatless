//! Last-chance workspace upload from an orphaned claim.
//!
//! The pre-stop `SyncWorkspace` RPC is best-effort: a pod that was evicted,
//! OOM-killed or already gone never answers it, and its volume is then the only
//! copy of everything since the last turn boundary. Reclaiming that claim on a
//! timer would delete work no snapshot holds.
//!
//! So before the cold transition deletes a claim it cannot prove is archived, it
//! runs one deterministic pod that mounts the claim, tars `WORKSPACE_DIR` and
//! `PUT`s it to a presigned URL. The pod carries no service-account token and no
//! AWS credentials — the URL is the whole grant, the same way a live sandbox
//! transfers its workspace.
//!
//! A one-shot Pod rather than a Job, matching [`super::node_archive`]'s cleanup
//! pod: the two have the same shape (run once, observe terminal phase, delete),
//! and the backend Role already carries `pods`.

use std::collections::BTreeMap;
use std::time::Duration;

use k8s_openapi::api::core::v1::{
    Container, EnvVar, PersistentVolumeClaimVolumeSource, Pod, PodSecurityContext, PodSpec,
    ResourceRequirements, SeccompProfile, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, DeleteParams, PostParams};
use tracing::{info, warn};

use super::client::K8sClient;
use super::naming::sanitize_k8s_name;
use moatless_core::config::Config;
use moatless_sandbox_runtime::runtime::RuntimeError;

/// Where the claim is mounted. The same path a sandbox pod uses, so
/// `WORKSPACE_DIR` needs no rewriting.
const WORKSPACE_MOUNT_PATH: &str = "/opt/moatless";
const WORKSPACE_DIR: &str = "/opt/moatless/workspace";

/// How long the rescue may take before the claim is left for the next cycle.
/// A large workspace tar plus one HTTPS transfer, with the sandbox's own
/// 5-minute transfer budget inside it.
const RESCUE_TIMEOUT: Duration = Duration::from_mins(10);

/// How often the pod's phase is polled.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

fn runtime_error(context: &str, error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Infrastructure(format!("{context}: {error}"))
}

pub(crate) fn rescue_pod_name(task_id: &str) -> String {
    let prefix = "moatless-rescue-";
    format!("{prefix}{}", sanitize_k8s_name(task_id, 63 - prefix.len()))
}

/// Tar the claim's workspace and upload it to `put_url`.
///
/// Returns `Ok(())` only when the pod reached `Succeeded`, which is what makes
/// the claim safe to delete. Every other outcome — a failed pod, a timeout, an
/// API error — is an error, and the caller keeps the claim.
pub(crate) async fn rescue_workspace(
    k8s: &K8sClient,
    config: &Config,
    task_id: &str,
    pvc_name: &str,
    put_url: &str,
) -> Result<(), RuntimeError> {
    let pods: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let name = rescue_pod_name(task_id);

    // A leftover pod from an earlier attempt would be observed instead of the
    // one carrying this run's presigned URL, and that URL expires. Delete first.
    delete_rescue_pod(&pods, task_id, &name).await?;

    let pod = build_rescue_pod(config, task_id, pvc_name, put_url);
    match pods.create(&PostParams::default(), &pod).await {
        Ok(_) => info!(task_id, pod_name = %name, "Created workspace rescue pod"),
        Err(kube::Error::Api(ref response)) if response.code == 409 => {}
        Err(error) => return Err(runtime_error("create workspace rescue pod", error)),
    }

    let result = await_rescue(&pods, task_id, &name).await;
    // Delete on every path: a succeeded pod is done and a failed one must not
    // outlive its presigned URL.
    delete_rescue_pod(&pods, task_id, &name).await?;
    result
}

async fn await_rescue(pods: &Api<Pod>, task_id: &str, name: &str) -> Result<(), RuntimeError> {
    let deadline = tokio::time::Instant::now() + RESCUE_TIMEOUT;
    loop {
        let current = pods
            .get_opt(name)
            .await
            .map_err(|error| runtime_error("observe workspace rescue pod", error))?;
        let Some(current) = current else {
            return Err(RuntimeError::Infrastructure(format!(
                "workspace rescue pod {name} vanished before completing"
            )));
        };
        match current
            .status
            .as_ref()
            .and_then(|status| status.phase.as_deref())
        {
            Some("Succeeded") => {
                info!(task_id, "Rescued workspace from orphaned claim");
                return Ok(());
            }
            Some("Failed") => {
                let detail = failure_detail(&current);
                return Err(RuntimeError::Infrastructure(format!(
                    "workspace rescue pod {name} failed: {detail}"
                )));
            }
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(RuntimeError::Infrastructure(format!(
                "workspace rescue pod {name} did not complete within {}s",
                RESCUE_TIMEOUT.as_secs()
            )));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Describe why a rescue pod's container exited non-zero.
///
/// The pod-level `status.message` Kubernetes sets only for pod-admission
/// failures, never for a container that ran and exited — that detail lives on
/// `containerStatuses[].state.terminated` instead, whose `message` field the
/// kubelet fills from the container's own stdout/stderr tail because the
/// container carries `terminationMessagePolicy: FallbackToLogsOnError`.
fn failure_detail(pod: &Pod) -> String {
    if let Some(message) = pod
        .status
        .as_ref()
        .and_then(|status| status.message.as_deref())
        .filter(|m| !m.is_empty())
    {
        return message.to_string();
    }
    let terminated = pod
        .status
        .as_ref()
        .and_then(|status| status.container_statuses.as_ref())
        .and_then(|statuses| statuses.iter().find(|cs| cs.name == "rescue"))
        .and_then(|cs| cs.state.as_ref())
        .and_then(|state| state.terminated.as_ref());
    let Some(terminated) = terminated else {
        return "unknown failure".to_string();
    };
    let reason = terminated.reason.as_deref().unwrap_or("Error");
    match terminated.message.as_deref().filter(|m| !m.is_empty()) {
        Some(message) => format!("{reason} (exit {}): {message}", terminated.exit_code),
        None => format!("{reason} (exit {})", terminated.exit_code),
    }
}

async fn delete_rescue_pod(pods: &Api<Pod>, task_id: &str, name: &str) -> Result<(), RuntimeError> {
    match pods.delete(name, &DeleteParams::default()).await {
        Ok(_) => {}
        Err(kube::Error::Api(ref response)) if response.code == 404 => {}
        Err(error) => return Err(runtime_error("delete workspace rescue pod", error)),
    }
    // The claim is RWO: a terminating rescue pod still holds it, and the delete
    // that follows would hang on the attachment.
    let deadline = tokio::time::Instant::now() + RESCUE_TIMEOUT;
    loop {
        match pods.get_opt(name).await {
            Ok(None) => return Ok(()),
            Ok(Some(_)) => {}
            Err(error) => return Err(runtime_error("observe workspace rescue pod delete", error)),
        }
        if tokio::time::Instant::now() >= deadline {
            warn!(task_id, pod_name = %name, "Workspace rescue pod did not terminate");
            return Err(RuntimeError::NotAbsent(format!(
                "workspace rescue pod {name} still present after delete"
            )));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

pub(crate) fn build_rescue_pod(
    config: &Config,
    task_id: &str,
    pvc_name: &str,
    put_url: &str,
) -> Pod {
    Pod {
        metadata: ObjectMeta {
            name: Some(rescue_pod_name(task_id)),
            labels: Some(BTreeMap::from([
                (
                    "moatless.io/task-id".to_string(),
                    sanitize_k8s_name(task_id, 63),
                ),
                (
                    "app.kubernetes.io/name".to_string(),
                    "moatless-workspace-rescue".to_string(),
                ),
                (
                    "app.kubernetes.io/managed-by".to_string(),
                    "moatless".to_string(),
                ),
                (
                    "app.kubernetes.io/component".to_string(),
                    "storage".to_string(),
                ),
            ])),
            ..Default::default()
        },
        spec: Some(PodSpec {
            // The presigned URL is the only grant this pod needs.
            automount_service_account_token: Some(false),
            restart_policy: Some("Never".to_string()),
            security_context: Some(PodSecurityContext {
                // The workspace tree is owned by UID 1000; read it as that user
                // rather than widening the pod's capabilities.
                run_as_user: Some(1000),
                run_as_group: Some(1000),
                fs_group: Some(1000),
                fs_group_change_policy: Some("OnRootMismatch".to_string()),
                seccomp_profile: Some(SeccompProfile {
                    type_: "RuntimeDefault".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            containers: vec![Container {
                name: "rescue".to_string(),
                image: Some(config.sandbox.image.clone()),
                image_pull_policy: Some(config.sandbox.image_pull_policy.clone()),
                // The image entrypoint launches a full sandbox; the binary's
                // one-shot mode is reached by invoking it directly.
                command: Some(vec!["/usr/local/bin/sandbox".to_string()]),
                // The pod is deleted immediately after `await_rescue` observes
                // a terminal phase, so the container's own stdout/stderr is the
                // only place a failure detail survives unless it rides along on
                // the status object itself.
                termination_message_policy: Some("FallbackToLogsOnError".to_string()),
                env: Some(vec![
                    EnvVar {
                        name: "SANDBOX_ONESHOT".to_string(),
                        value: Some("sync".to_string()),
                        ..Default::default()
                    },
                    EnvVar {
                        name: "WORKSPACE_SYNC_PUT_URL".to_string(),
                        value: Some(put_url.to_string()),
                        ..Default::default()
                    },
                    EnvVar {
                        name: "WORKSPACE_DIR".to_string(),
                        value: Some(WORKSPACE_DIR.to_string()),
                        ..Default::default()
                    },
                    EnvVar {
                        name: "TASK_ID".to_string(),
                        value: Some(task_id.to_string()),
                        ..Default::default()
                    },
                ]),
                resources: Some(ResourceRequirements {
                    requests: Some(BTreeMap::from([
                        (
                            "cpu".to_string(),
                            Quantity(config.sandbox.resources_cpu_request.clone()),
                        ),
                        (
                            "memory".to_string(),
                            Quantity(config.sandbox.resources_memory_request.clone()),
                        ),
                    ])),
                    limits: Some(BTreeMap::from([
                        (
                            "cpu".to_string(),
                            Quantity(config.sandbox.resources_cpu_limit.clone()),
                        ),
                        (
                            "memory".to_string(),
                            Quantity(config.sandbox.resources_memory_limit.clone()),
                        ),
                    ])),
                    ..Default::default()
                }),
                volume_mounts: Some(vec![VolumeMount {
                    name: "workspace-data".to_string(),
                    mount_path: WORKSPACE_MOUNT_PATH.to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            }],
            volumes: Some(vec![Volume {
                name: "workspace-data".to_string(),
                persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                    claim_name: pvc_name.to_string(),
                    read_only: Some(false),
                }),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config::test_minimal()
    }

    #[test]
    fn rescue_pod_mounts_the_claim_and_carries_the_presigned_url() {
        let pod = build_rescue_pod(
            &test_config(),
            "task-1",
            "moatless-pvc-task-1",
            "https://s3/put",
        );
        let spec = pod.spec.expect("spec");

        let volume = spec.volumes.expect("volumes").into_iter().next().unwrap();
        assert_eq!(
            volume
                .persistent_volume_claim
                .expect("pvc source")
                .claim_name,
            "moatless-pvc-task-1"
        );

        let container = spec.containers.into_iter().next().unwrap();
        assert!(
            container
                .volume_mounts
                .expect("mounts")
                .iter()
                .any(|m| m.name == "workspace-data" && m.mount_path == WORKSPACE_MOUNT_PATH)
        );
        let env = container.env.expect("env");
        assert_eq!(
            env.iter()
                .find(|e| e.name == "SANDBOX_ONESHOT")
                .and_then(|e| e.value.as_deref()),
            Some("sync")
        );
        assert_eq!(
            env.iter()
                .find(|e| e.name == "WORKSPACE_SYNC_PUT_URL")
                .and_then(|e| e.value.as_deref()),
            Some("https://s3/put")
        );
    }

    #[test]
    fn rescue_pod_carries_no_credentials_of_its_own() {
        let pod = build_rescue_pod(&test_config(), "task-1", "pvc", "https://s3/put");
        let spec = pod.spec.expect("spec");
        assert_eq!(spec.automount_service_account_token, Some(false));
        assert_eq!(spec.restart_policy.as_deref(), Some("Never"));

        let container = spec.containers.into_iter().next().unwrap();
        let env = container.env.expect("env");
        // No task token, no user token, no AWS credentials — the presigned URL
        // is the whole grant.
        assert!(
            !env.iter()
                .any(|e| e.name.contains("TOKEN") || e.name.starts_with("AWS_"))
        );
    }

    #[test]
    fn rescue_pod_requests_and_limits_are_set() {
        let pod = build_rescue_pod(&test_config(), "task-1", "pvc", "https://s3/put");
        let container = pod
            .spec
            .expect("spec")
            .containers
            .into_iter()
            .next()
            .unwrap();
        let resources = container.resources.expect("resources");
        assert!(resources.requests.expect("requests").contains_key("memory"));
        assert!(resources.limits.expect("limits").contains_key("memory"));
    }

    #[test]
    fn rescue_pod_name_fits_the_k8s_limit() {
        let name = rescue_pod_name(&"t".repeat(120));
        assert!(name.len() <= 63, "name was {} chars", name.len());
        assert!(name.starts_with("moatless-rescue-"));
    }

    #[test]
    fn rescue_pod_falls_back_to_logs_on_error() {
        let pod = build_rescue_pod(&test_config(), "task-1", "pvc", "https://s3/put");
        let container = pod
            .spec
            .expect("spec")
            .containers
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(
            container.termination_message_policy.as_deref(),
            Some("FallbackToLogsOnError")
        );
    }

    fn pod_with_terminated_rescue_container(
        terminated: k8s_openapi::api::core::v1::ContainerStateTerminated,
    ) -> Pod {
        use k8s_openapi::api::core::v1::{ContainerState, ContainerStatus, PodStatus};

        Pod {
            status: Some(PodStatus {
                phase: Some("Failed".to_string()),
                container_statuses: Some(vec![ContainerStatus {
                    name: "rescue".to_string(),
                    state: Some(ContainerState {
                        terminated: Some(terminated),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn failure_detail_reads_the_container_terminated_state() {
        use k8s_openapi::api::core::v1::ContainerStateTerminated;

        let pod = pod_with_terminated_rescue_container(ContainerStateTerminated {
            exit_code: 101,
            reason: Some("Error".to_string()),
            message: None,
            ..Default::default()
        });
        assert_eq!(failure_detail(&pod), "Error (exit 101)");
    }

    #[test]
    fn failure_detail_includes_the_termination_message_when_present() {
        use k8s_openapi::api::core::v1::ContainerStateTerminated;

        let pod = pod_with_terminated_rescue_container(ContainerStateTerminated {
            exit_code: 101,
            reason: Some("Error".to_string()),
            message: Some("BACKEND_GRPC_URL is required: NotPresent".to_string()),
            ..Default::default()
        });
        assert_eq!(
            failure_detail(&pod),
            "Error (exit 101): BACKEND_GRPC_URL is required: NotPresent"
        );
    }

    #[test]
    fn failure_detail_falls_back_when_no_container_status_is_observed() {
        let pod = Pod::default();
        assert_eq!(failure_detail(&pod), "unknown failure");
    }
}
