//! Unified error type for sandbox operations.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use moatless_backend_core::sandbox as core_sandbox;

use moatless_core::error::error_response;

/// Errors that can occur during sandbox lifecycle operations.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("Configuration missing: {0}")]
    ConfigMissing(String),

    /// A synthesized JWT complaint with no underlying failure. Minting
    /// failures use [`SandboxError::JwtSigning`].
    #[error("JWT generation failed: {0}")]
    JwtError(String),

    /// Minting a sandbox or user token failed. Retains the jsonwebtoken error
    /// — the key material it complains about never leaves the log line.
    #[error("JWT generation failed for {purpose}")]
    JwtSigning {
        purpose: &'static str,
        #[source]
        source: jsonwebtoken::errors::Error,
    },

    /// A Kubernetes API call failed. The `kube` error is boxed at the provider
    /// call site so this band's error type names no provider SDK — the orphan
    /// rule forbids a `From<kube::Error>` here anyway, since both types are
    /// foreign to band 7. Boxed rather than stringified so the cause stays on
    /// the source chain for `causal_chain`.
    #[error("Kubernetes API error: {0}")]
    KubeError(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("Pod build failed: {0}")]
    PodBuildError(String),

    /// A synthesized Docker complaint (an inspect response missing a field, a
    /// container that outlived its removal). Engine call failures use
    /// [`SandboxError::Docker`].
    #[error("Docker API error: {0}")]
    DockerError(String),

    /// A Docker Engine call failed. `operation` names the call so the log says
    /// what was attempted without the caller having to re-describe it. The
    /// `bollard` error is boxed at the provider call site so this band's error
    /// type names no provider SDK; boxed rather than stringified so the cause
    /// stays on the source chain for `causal_chain`.
    #[error("Docker API error during {operation}")]
    Docker {
        operation: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Re-encoding an inspected container's config so it can be recreated
    /// failed.
    #[error("Docker container config could not be rebuilt")]
    DockerConfig(#[source] serde_json::Error),

    /// A gRPC call to a sandbox failed with a status this backend does not
    /// classify further. Statuses that map onto a client-visible outcome
    /// (unavailable, not-found, invalid-argument) stay in their own variants.
    #[error("gRPC {operation} failed")]
    Grpc {
        operation: String,
        #[source]
        source: tonic::Status,
    },

    /// The sandbox answered that it does not serve the method that was called:
    /// its image predates it. Backend and sandbox images roll out separately,
    /// so this is a version skew a redeploy resolves, and it is worth telling
    /// apart from a sandbox that could not be reached at all — the request got
    /// there, and waiting will not change the answer.
    #[error("Sandbox does not implement {operation}")]
    Unsupported {
        operation: String,
        #[source]
        source: tonic::Status,
    },

    /// The gRPC endpoint for a sandbox could not be built.
    #[error("Sandbox gRPC client could not be created")]
    GrpcClient(#[from] crate::sandbox::client::ClientError),

    /// A database read backing a sandbox operation failed. `context` names the
    /// read; the SQL error rides along as the source.
    #[error("Database error while {context}")]
    Database {
        context: &'static str,
        #[source]
        source: sqlx::Error,
    },

    /// The authorization check for a sandbox request could not be evaluated.
    #[error("Sandbox authorization check failed")]
    Authorization(#[from] moatless_backend_core::access_control::AccessControlError),

    /// A runtime step raised by a provider's own helpers (node-archive
    /// bookkeeping, log streaming). Kept as a source so `map_sandbox_error`
    /// can hand lifecycle back the original classification instead of a
    /// flattened string.
    #[error("Sandbox runtime step failed")]
    Runtime(#[source] crate::sandbox::lifecycle::RuntimeError),

    /// A lifecycle operation failed for a reason with no client-visible
    /// mapping. The three that *do* map — task-not-found, closed-task, and
    /// lock contention — become `NotFound`/`BadRequest`/`Conflict` instead.
    #[error("Sandbox lifecycle operation failed")]
    Lifecycle(#[source] Box<crate::sandbox::lifecycle::LifecycleError>),

    #[error("Sandbox not ready: {0}")]
    NotReady(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    /// A sandbox response carried a protobuf value this backend cannot
    /// represent. Retains the conversion failure as its source so the offending
    /// wire value survives to the log.
    #[error("Invalid sandbox response: {0}")]
    InvalidResponse(#[from] crate::sandbox::schemas::UnknownFileNodeType),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<crate::sandbox::lifecycle::LifecycleError> for SandboxError {
    fn from(value: crate::sandbox::lifecycle::LifecycleError) -> Self {
        use crate::sandbox::lifecycle::LifecycleError as LE;
        match &value {
            LE::TaskNotFound(id) => Self::NotFound(id.clone()),
            // The Closed guard rejected the operation — a client state error.
            LE::NotAllowed { .. } => Self::BadRequest(value.to_string()),
            // Another operation holds the per-Task lock — a client-visible
            // conflict (409), matching the task API.
            LE::OperationInProgress { .. } => Self::Conflict(value.to_string()),
            // Everything else is an internal failure: keep the lifecycle error
            // itself so the response boundary can log the whole chain,
            // including the `RuntimeError` a failed effect carries.
            _ => Self::Lifecycle(Box::new(value)),
        }
    }
}

impl SandboxError {
    /// This module's error as the sandbox runtime contract states it.
    ///
    /// Every variant is named, so a variant added here does not compile until
    /// its contract mapping is decided. The two container runtimes reach the
    /// contract as a [`core_sandbox::ProviderKind`] and a message, so nothing
    /// downstream of the contract compiles `kube` or `bollard`. The backend-only
    /// outcomes — configuration, token minting, the database, lifecycle, and the
    /// HTTP-only 409/403 the contract does not model — flatten to `Other` with
    /// their rendered text intact.
    pub(crate) fn into_contract(self) -> core_sandbox::SandboxError {
        use core_sandbox::ProviderKind;

        let rendered = self.to_string();
        match self {
            Self::NotReady(message) => core_sandbox::SandboxError::NotReady(message),
            Self::NotFound(message) => core_sandbox::SandboxError::NotFound(message),
            Self::BadRequest(message) => core_sandbox::SandboxError::BadRequest(message),
            Self::Unsupported { operation, source } => {
                core_sandbox::SandboxError::Unsupported { operation, source }
            }
            Self::Grpc { operation, source } => {
                core_sandbox::SandboxError::Grpc { operation, source }
            }
            Self::GrpcClient(source) => core_sandbox::SandboxError::GrpcClient(source.to_string()),
            Self::KubeError(_) | Self::PodBuildError(_) => core_sandbox::SandboxError::Provider {
                kind: ProviderKind::Kubernetes,
                message: rendered,
            },
            Self::DockerError(_) | Self::Docker { .. } | Self::DockerConfig(_) => {
                core_sandbox::SandboxError::Provider {
                    kind: ProviderKind::Docker,
                    message: rendered,
                }
            }
            Self::ConfigMissing(_)
            | Self::JwtError(_)
            | Self::JwtSigning { .. }
            | Self::Database { .. }
            | Self::Authorization(_)
            | Self::Runtime(_)
            | Self::Lifecycle(_)
            | Self::Conflict(_)
            | Self::Forbidden(_)
            | Self::InvalidResponse(_)
            | Self::Internal(_) => core_sandbox::SandboxError::Other(rendered),
        }
    }
}

impl From<core_sandbox::SandboxError> for SandboxError {
    /// The contract error as this module's error.
    ///
    /// Every variant is named, so a variant added to the contract does not
    /// compile until its module mapping is decided. A provider-classified or
    /// client-construction failure has already been flattened past the point
    /// where the runtime that raised it could be reconstructed, so both — and
    /// the contract's `Other` — arrive as an internal failure with their text
    /// intact.
    fn from(error: core_sandbox::SandboxError) -> Self {
        use core_sandbox::SandboxError as Contract;

        let rendered = error.to_string();
        match error {
            Contract::NotReady(message) => Self::NotReady(message),
            Contract::NotFound(message) => Self::NotFound(message),
            Contract::BadRequest(message) => Self::BadRequest(message),
            Contract::Unsupported { operation, source } => Self::Unsupported { operation, source },
            Contract::Grpc { operation, source } => Self::Grpc { operation, source },
            Contract::GrpcClient(_) | Contract::Provider { .. } | Contract::Other(_) => {
                Self::Internal(rendered)
            }
        }
    }
}

impl IntoResponse for SandboxError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::NotReady(_) => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            Self::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Conflict(_) => (StatusCode::CONFLICT, self.to_string()),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            Self::ConfigMissing(ref msg) => {
                tracing::error!(error = %msg, "Sandbox config missing");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::JwtError(ref msg) => {
                tracing::error!(error = %msg, "Sandbox JWT error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::PodBuildError(ref msg) => {
                tracing::error!(error = %msg, "Sandbox pod build error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::KubeError(_) => {
                tracing::error!(
                    error = %moatless_core::error::causal_chain(&self),
                    "Kubernetes API error in sandbox handler"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::DockerError(ref msg) => {
                tracing::error!(error = %msg, "Docker API error in sandbox handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            // Every source-bearing variant renders its whole chain into one
            // log line here — the single observability boundary — and answers
            // with the same redacted body.
            Self::JwtSigning { .. }
            | Self::Docker { .. }
            | Self::DockerConfig(_)
            | Self::Grpc { .. }
            | Self::Unsupported { .. }
            | Self::GrpcClient(_)
            | Self::Database { .. }
            | Self::Authorization(_)
            | Self::Runtime(_)
            | Self::Lifecycle(_) => {
                tracing::error!(
                    error = %moatless_core::error::causal_chain(&self),
                    "Sandbox operation failed"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::InvalidResponse(ref e) => {
                tracing::error!(error = %e, "Invalid sandbox gRPC response");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::Internal(ref msg) => {
                tracing::error!(error = %msg, "Internal sandbox error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };
        error_response(status, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_sandbox::{ProviderKind, SandboxError as ContractError};

    /// The contract must be satisfiable without compiling either container
    /// runtime, so every `kube`- and `bollard`-shaped failure has to arrive as a
    /// kind and a message.
    #[test]
    fn container_runtime_failures_reach_the_contract_provider_classified() {
        let cases = vec![
            (
                SandboxError::KubeError("pods \"sandbox-t-1\" not found".into()),
                ProviderKind::Kubernetes,
            ),
            (
                SandboxError::PodBuildError("no node has 4 CPUs free".into()),
                ProviderKind::Kubernetes,
            ),
            (
                SandboxError::DockerError("inspect response had no config".into()),
                ProviderKind::Docker,
            ),
            (
                SandboxError::Docker {
                    operation: "creating the sandbox container",
                    source: "/var/run/docker.sock: permission denied".into(),
                },
                ProviderKind::Docker,
            ),
            (
                SandboxError::DockerConfig(
                    serde_json::from_str::<serde_json::Value>("{not json").unwrap_err(),
                ),
                ProviderKind::Docker,
            ),
        ];

        for (error, expected) in cases {
            let rendered = error.to_string();
            match error.into_contract() {
                ContractError::Provider { kind, message } => {
                    assert_eq!(kind, expected, "{rendered}");
                    assert_eq!(message, rendered);
                }
                other => panic!("{rendered} classified as {other:?}"),
            }
        }
    }

    /// The transport outcomes a caller acts on — 404, 400, a sandbox too old to
    /// serve a method — keep their shape across the boundary rather than
    /// flattening into a string.
    #[test]
    fn transport_outcomes_keep_their_contract_shape() {
        assert!(matches!(
            SandboxError::NotReady("starting".into()).into_contract(),
            ContractError::NotReady(m) if m == "starting"
        ));
        assert!(matches!(
            SandboxError::NotFound("src/mystery".into()).into_contract(),
            ContractError::NotFound(m) if m == "src/mystery"
        ));
        assert!(matches!(
            SandboxError::BadRequest("path is a directory".into()).into_contract(),
            ContractError::BadRequest(m) if m == "path is a directory"
        ));
        assert!(matches!(
            SandboxError::Unsupported {
                operation: "list_git_refs".into(),
                source: tonic::Status::unimplemented("unknown service"),
            }
            .into_contract(),
            ContractError::Unsupported { operation, .. } if operation == "list_git_refs"
        ));
        assert!(matches!(
            SandboxError::Grpc {
                operation: "read_file".into(),
                source: tonic::Status::internal("boom"),
            }
            .into_contract(),
            ContractError::Grpc { operation, .. } if operation == "read_file"
        ));
    }

    /// The backend-local outcomes — configuration, the database, lifecycle, and
    /// the two HTTP-only statuses — have no contract shape, and flatten with
    /// their text intact.
    #[test]
    fn backend_local_failures_flatten_with_their_message() {
        for error in [
            SandboxError::ConfigMissing("TASK_TOKEN_SECRET".into()),
            SandboxError::JwtError("missing signing secret".into()),
            SandboxError::Database {
                context: "reading the task row for provisioning",
                source: sqlx::Error::Protocol("connection reset".to_string()),
            },
            SandboxError::Conflict("another operation is in progress".into()),
            SandboxError::Forbidden("no access to task".into()),
            SandboxError::Internal("crash".into()),
        ] {
            let rendered = error.to_string();
            match error.into_contract() {
                ContractError::Other(message) => assert_eq!(message, rendered),
                other => panic!("{rendered} classified as {other:?}"),
            }
        }
    }

    #[test]
    fn error_display_messages() {
        assert_eq!(
            SandboxError::ConfigMissing("TOKEN_SECRET".into()).to_string(),
            "Configuration missing: TOKEN_SECRET"
        );
        assert_eq!(
            SandboxError::JwtError("expired".into()).to_string(),
            "JWT generation failed: expired"
        );
        assert_eq!(
            SandboxError::PodBuildError("oom".into()).to_string(),
            "Pod build failed: oom"
        );
        assert_eq!(
            SandboxError::NotReady("starting".into()).to_string(),
            "Sandbox not ready: starting"
        );
        assert_eq!(
            SandboxError::Internal("crash".into()).to_string(),
            "Internal error: crash"
        );
    }

    #[tokio::test]
    async fn not_ready_returns_503() {
        let resp = SandboxError::NotReady("still starting".into()).into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn not_found_returns_404() {
        let resp = SandboxError::NotFound("missing.txt".into()).into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn bad_request_returns_400() {
        let resp = SandboxError::BadRequest("Path is a directory: subdir".into()).into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn forbidden_returns_403() {
        let resp = SandboxError::Forbidden("no access to task".into()).into_response();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn config_missing_returns_500() {
        let resp = SandboxError::ConfigMissing("secret".into()).into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn internal_returns_500() {
        let resp = SandboxError::Internal("boom".into()).into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn invalid_response_returns_500_without_leaking_the_wire_value() {
        let err = SandboxError::from(crate::sandbox::schemas::UnknownFileNodeType {
            path: "src/mystery".into(),
            node_type: 42,
        });
        // The conversion failure stays reachable as a source for logging.
        let source = std::error::Error::source(&err).expect("conversion error is the source");
        assert_eq!(
            source.to_string(),
            "unknown FileNodeType 42 for file tree node 'src/mystery'"
        );

        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"].as_str().unwrap(), "Internal server error");
    }

    /// Every source-bearing variant, built from a cause whose `Display`
    /// carries exactly the detail the client must never see.
    fn source_bearing_errors() -> Vec<(SandboxError, &'static str)> {
        vec![
            (
                SandboxError::JwtSigning {
                    purpose: "the sandbox task token",
                    source: jsonwebtoken::errors::ErrorKind::InvalidEcdsaKey.into(),
                },
                "InvalidEcdsaKey",
            ),
            (
                SandboxError::Docker {
                    operation: "creating the sandbox container",
                    source: "/var/run/docker.sock: permission denied".into(),
                },
                "docker.sock",
            ),
            (
                SandboxError::DockerConfig(
                    serde_json::from_str::<serde_json::Value>("{not json").unwrap_err(),
                ),
                "key must be a string",
            ),
            (
                SandboxError::Grpc {
                    operation: "execute_agent".to_string(),
                    source: tonic::Status::internal("agent bearer token rejected"),
                },
                "bearer token",
            ),
            (
                SandboxError::Database {
                    context: "reading the task row for provisioning",
                    source: sqlx::Error::Protocol("SELECT * FROM tasks WHERE id = $1".to_string()),
                },
                "SELECT * FROM tasks",
            ),
            (
                SandboxError::Runtime(crate::sandbox::lifecycle::RuntimeError::Infrastructure(
                    "/data/sandboxes/task-1 is not writable".to_string(),
                )),
                "/data/sandboxes/task-1",
            ),
            (
                SandboxError::Lifecycle(Box::new(
                    crate::sandbox::lifecycle::LifecycleError::IntentWrite {
                        task_id: "task-1".to_string(),
                        reason: "UPDATE sandboxes SET desired_state = $1".to_string(),
                    },
                )),
                "UPDATE sandboxes",
            ),
        ]
    }

    #[test]
    fn every_wrapping_variant_keeps_its_cause_on_the_source_chain() {
        for (error, needle) in source_bearing_errors() {
            let source =
                std::error::Error::source(&error).unwrap_or_else(|| panic!("no source: {error:?}"));
            assert!(
                moatless_core::error::causal_chain(source).contains(needle),
                "cause {needle:?} lost from {error:?}"
            );
        }
    }

    #[tokio::test]
    async fn wrapping_variants_respond_500_with_no_grpc_sql_docker_or_key_detail() {
        for (error, needle) in source_bearing_errors() {
            let resp = error.into_response();
            assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
            let body = http_body_util::BodyExt::collect(resp.into_body())
                .await
                .unwrap()
                .to_bytes();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(
                json,
                serde_json::json!({ "error": "Internal server error" })
            );
            assert!(!json.to_string().contains(needle));
        }
    }

    #[test]
    fn synthesized_variants_have_no_source() {
        for error in [
            SandboxError::Internal("lifecycle unavailable (no database)".into()),
            SandboxError::JwtError("missing signing secret".into()),
            SandboxError::DockerError("Container inspect response had no config".into()),
            SandboxError::ConfigMissing("TASK_TOKEN_SECRET".into()),
        ] {
            assert!(
                std::error::Error::source(&error).is_none(),
                "{error:?} should stay a synthesized message"
            );
        }
    }

    #[tokio::test]
    async fn internal_errors_do_not_leak_details() {
        let resp = SandboxError::ConfigMissing("TOKEN_SECRET".into()).into_response();
        let body = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Should NOT expose the actual config key
        let error_msg = json["error"].as_str().unwrap();
        assert!(!error_msg.contains("TOKEN_SECRET"));
        assert_eq!(error_msg, "Internal server error");
    }
}
